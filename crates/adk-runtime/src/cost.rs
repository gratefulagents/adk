use adk_core::{RunResult, Usage};

use crate::CostEstimator;

pub(crate) fn estimate_result_cost(
    result: &RunResult,
    model: &str,
    estimator: Option<&dyn CostEstimator>,
) -> (f64, bool) {
    let summary = result
        .metrics
        .as_ref()
        .and_then(|metrics| metrics.response_cost.as_ref());
    let Some(summary) = summary else {
        let cost = result
            .metrics
            .as_ref()
            .map_or(0.0, |metrics| metrics.cost_usd);
        return (
            if cost.is_finite() && cost >= 0.0 {
                cost
            } else {
                0.0
            },
            false,
        );
    };
    let (mut cost, mut known) = if result.responses.is_empty() {
        (0.0, summary.cost_known)
    } else if summary.cost_usd.is_finite() && summary.cost_usd >= 0.0 {
        (summary.cost_usd, summary.cost_known)
    } else {
        (0.0, false)
    };
    let mut residual = result.usage.clone();
    residual.context_tokens = None;
    for response in &result.responses {
        let used = &response.usage;
        residual.requests = residual.requests.saturating_sub(used.requests);
        residual.input_tokens = residual.input_tokens.saturating_sub(used.input_tokens);
        residual.output_tokens = residual.output_tokens.saturating_sub(used.output_tokens);
        residual.cache_read_tokens = residual
            .cache_read_tokens
            .saturating_sub(used.cache_read_tokens);
        residual.cache_creation_tokens = residual
            .cache_creation_tokens
            .saturating_sub(used.cache_creation_tokens);
    }
    if result.responses.is_empty() || has_tokens(&residual) {
        match estimator.map(|estimator| estimator.estimate_display_cost(model, &residual)) {
            Some(price)
                if price.cost_usd.is_finite()
                    && price.cost_usd >= 0.0
                    && (cost + price.cost_usd).is_finite() =>
            {
                cost += price.cost_usd;
                known &= price.cost_known;
            }
            _ => known = false,
        }
    }
    (cost, known)
}

fn has_tokens(usage: &Usage) -> bool {
    usage.input_tokens != 0
        || usage.output_tokens != 0
        || usage.cache_read_tokens != 0
        || usage.cache_creation_tokens != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use adk_core::{ModelResponse, ResponseCostSummary, RunMetrics, RunStatus};
    use std::sync::Mutex;

    struct Price {
        amount: f64,
        known: bool,
        calls: Mutex<Vec<(String, Usage)>>,
    }
    impl CostEstimator for Price {
        fn cost(&self, _: &str, _: &Usage) -> f64 {
            panic!("display projection must not charge accounting")
        }
        fn estimate_display_cost(&self, model: &str, usage: &Usage) -> ResponseCostSummary {
            self.calls
                .lock()
                .unwrap()
                .push((model.into(), usage.clone()));
            ResponseCostSummary {
                cost_usd: self.amount,
                cost_known: self.known,
            }
        }
    }
    fn price(amount: f64, known: bool) -> Price {
        Price {
            amount,
            known,
            calls: Mutex::new(vec![]),
        }
    }
    fn result(usage: Usage, responses: Vec<Usage>, subtotal: Option<(f64, bool)>) -> RunResult {
        RunResult {
            action_audit_records: vec![],
            metrics: Some(RunMetrics {
                model: Some("active".into()),
                turns: 2,
                cost_usd: 999.0,
                cost_known: false,
                response_cost: subtotal.map(|(cost_usd, cost_known)| ResponseCostSummary {
                    cost_usd,
                    cost_known,
                }),
                elapsed_ms: 0,
            }),
            status: RunStatus::Completed,
            final_output: None,
            final_output_is_raw_json: false,
            new_items: vec![],
            new_items_provenance: vec![],
            history: vec![],
            history_provenance: vec![],
            responses: responses
                .into_iter()
                .map(|usage| ModelResponse {
                    usage,
                    items: vec![],
                    end_turn: None,
                    response_id: None,
                    metadata: Default::default(),
                    raw: None,
                    snapshot_raw: None,
                    snapshot_projection: None,
                })
                .collect(),
            usage,
            pending_approvals: vec![],
            last_agent: None,
            guardrails: vec![],
        }
    }

    #[test]
    fn residuals_clamp_each_counter_preserve_requests_and_ignore_context() {
        let total = Usage {
            requests: 5,
            input_tokens: 2,
            output_tokens: 9,
            cache_read_tokens: 7,
            cache_creation_tokens: 1,
            context_tokens: Some(200),
        };
        let response = Usage {
            requests: 2,
            input_tokens: 8,
            output_tokens: 4,
            cache_read_tokens: 3,
            cache_creation_tokens: 6,
            context_tokens: Some(10),
        };
        let result = result(
            total.clone(),
            vec![response.clone(), response],
            Some((0.75, true)),
        );
        let price = price(1.0, true);
        assert_eq!(
            estimate_result_cost(&result, "original", Some(&price)),
            (1.75, true)
        );
        assert_eq!(
            *price.calls.lock().unwrap(),
            vec![(
                "original".into(),
                Usage {
                    requests: 1,
                    output_tokens: 1,
                    cache_read_tokens: 1,
                    ..Default::default()
                }
            )]
        );
        assert_eq!(result.usage, total);
    }

    #[test]
    fn cache_only_residuals_are_priced_but_requests_only_are_not() {
        for (read, create, priced) in [(0, 0, false), (2, 0, true), (0, 3, true)] {
            let result = result(
                Usage {
                    requests: 9,
                    cache_read_tokens: read,
                    cache_creation_tokens: create,
                    context_tokens: Some(99),
                    ..Default::default()
                },
                vec![Usage::default()],
                Some((0.75, true)),
            );
            let price = price(0.25, true);
            assert_eq!(
                estimate_result_cost(&result, "original", Some(&price)),
                (if priced { 1.0 } else { 0.75 }, true)
            );
            assert_eq!(price.calls.lock().unwrap().len(), usize::from(priced));
        }
    }

    #[test]
    fn empty_responses_price_even_zero_usage_with_original_model() {
        for usage in [
            Usage::default(),
            Usage {
                requests: 2,
                input_tokens: 5,
                context_tokens: Some(99),
                ..Default::default()
            },
        ] {
            let result = result(usage.clone(), vec![], Some((99.0, true)));
            let price = price(0.0, true);
            assert_eq!(
                estimate_result_cost(&result, "original", Some(&price)),
                (0.0, true)
            );
            assert_eq!(
                *price.calls.lock().unwrap(),
                vec![(
                    "original".into(),
                    Usage {
                        context_tokens: None,
                        ..usage
                    }
                )]
            );
        }
    }

    #[test]
    fn uncertain_nonzero_fallback_and_frozen_prices_keep_amounts() {
        for known in [false, true] {
            let result = result(
                Usage {
                    input_tokens: 1,
                    ..Default::default()
                },
                vec![Usage::default()],
                Some((0.75, known)),
            );
            let price = price(0.25, false);
            assert_eq!(
                estimate_result_cost(&result, "original", Some(&price)),
                (1.0, false)
            );
        }
    }

    #[test]
    fn missing_provenance_never_becomes_known_or_reprices_responses() {
        for responses in [
            vec![],
            vec![Usage {
                input_tokens: 1,
                ..Default::default()
            }],
        ] {
            let result = result(
                Usage {
                    input_tokens: 1,
                    ..Default::default()
                },
                responses,
                None,
            );
            let price = price(0.25, true);
            assert_eq!(
                estimate_result_cost(&result, "original", Some(&price)),
                (999.0, false)
            );
            assert!(price.calls.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn invalid_or_overflowing_projection_retains_subtotal_and_usage() {
        for amount in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
            let result = result(
                Usage {
                    input_tokens: 1,
                    ..Default::default()
                },
                vec![Usage::default()],
                Some((f64::MAX, true)),
            );
            let price = price(amount, true);
            assert_eq!(
                estimate_result_cost(&result, "original", Some(&price)),
                (f64::MAX, false)
            );
            assert_eq!(result.usage.input_tokens, 1);
        }
    }

    #[test]
    fn no_estimator_does_not_invalidate_frozen_fully_covered_prices() {
        let result = result(
            Usage::default(),
            vec![Usage::default()],
            Some((0.0000004, true)),
        );
        assert_eq!(
            estimate_result_cost(&result, "original", None),
            (0.0000004, true)
        );
    }

    #[test]
    fn sdk_result_cost_matches_pinned_go_oracle() {
        struct LegacyPrice(Price);
        impl CostEstimator for LegacyPrice {
            fn cost(&self, model: &str, usage: &Usage) -> f64 {
                self.0
                    .calls
                    .lock()
                    .unwrap()
                    .push((model.into(), usage.clone()));
                self.0.amount
            }
        }
        fn usage(value: &serde_json::Value) -> Usage {
            Usage {
                requests: value["requests"].as_u64().unwrap_or(0),
                input_tokens: value["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: value["output_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: value["cache_read_tokens"].as_u64().unwrap_or(0),
                cache_creation_tokens: value["cache_create_tokens"].as_u64().unwrap_or(0),
                context_tokens: None,
            }
        }
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../fixtures/costs/sdk-result-cost.json"))
                .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let responses = case["responses"].as_array().cloned().unwrap_or_default();
            let subtotal = responses
                .iter()
                .fold((0.0, true), |(cost, known), response| {
                    (
                        cost + response["cost_usd"].as_f64().unwrap(),
                        known && response["cost_known"].as_bool().unwrap(),
                    )
                });
            let result = result(
                usage(&case["usage"]),
                responses
                    .iter()
                    .map(|response| usage(&response["usage"]))
                    .collect(),
                Some(subtotal),
            );
            let price = price(
                case["fallback_cost"].as_f64().unwrap(),
                case["fallback_kind"] != "unknown",
            );
            let legacy = LegacyPrice(price);
            let estimator: Option<&dyn CostEstimator> =
                match case["fallback_kind"].as_str().unwrap() {
                    "nil" => None,
                    "legacy" => Some(&legacy),
                    "known" | "unknown" => Some(&legacy.0),
                    kind => panic!("unknown fixture kind {kind}"),
                };
            assert_eq!(
                estimate_result_cost(&result, "original", estimator),
                (
                    case["cost_usd"].as_f64().unwrap(),
                    case["cost_known"].as_bool().unwrap()
                ),
                "{name}"
            );
            let expected: Vec<_> = case["fallback_usages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| ("original".into(), usage(value)))
                .collect();
            assert_eq!(*legacy.0.calls.lock().unwrap(), expected, "{name}");
        }
    }
}
