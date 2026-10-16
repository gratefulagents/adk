//! Exact tables from `repos/sdk/internal/openai/cost_test.go` at
//! `1dc92b73900fac74dc357a938e4b5eee6392b418` (TestCalculateCost / TestEstimateCost).
//! Case names, counters, expected costs and known flags are source literals, not
//! expectations calculated from the Rust pricing catalog. The Go tolerance is
//! abs(got - want) <= 1e-9; these assertions also reject NaN.

use adk_core::Usage;
use adk_providers::{cost, wire, wire::Protocol};
use serde_json::json;

// Go anthropic.Usage fields map to OpenAI Responses wire counters as follows:
// InputTokens -> input_tokens; OutputTokens -> output_tokens;
// CacheReadInputTokens -> input_tokens_details.cached_tokens;
// CacheCreationInputTokens -> input_tokens_details.cache_write_tokens.
// The latter two are subsets of input, not additional prompt tokens.
fn usage(input: u64, output: u64, cache_read: u64, cache_creation: u64) -> Usage {
    wire::usage(
        &json!({
            "input_tokens": input,
            "output_tokens": output,
            "input_tokens_details": {
                "cached_tokens": cache_read,
                "cache_write_tokens": cache_creation,
            },
        }),
        Protocol::Responses,
    )
}

#[test]
fn openai_calculate_cost_original_table() {
    // (Go subtest name, model, usage, wantCost), in original source order.
    let cases = [
        ("gpt-4.1 input", "gpt-4.1", usage(1_000_000, 0, 0, 0), 2.0),
        ("gpt-4.1 output", "gpt-4.1", usage(0, 1_000_000, 0, 0), 8.0),
        (
            "gpt-4.1-mini mixed",
            "gpt-4.1-mini",
            usage(2000, 1000, 0, 0),
            0.0024,
        ),
        (
            "gpt-5.3-codex input uses corrected rate",
            "gpt-5.3-codex",
            usage(1_000_000, 0, 0, 0),
            1.75,
        ),
        (
            "gpt-5.3-codex output uses corrected rate",
            "gpt-5.3-codex",
            usage(0, 1_000_000, 0, 0),
            14.0,
        ),
        (
            "gpt-5.4-nano input uses corrected rate",
            "gpt-5.4-nano",
            usage(1_000_000, 0, 0, 0),
            0.2,
        ),
        (
            "gpt-5.4-nano output uses corrected rate",
            "gpt-5.4-nano",
            usage(0, 1_000_000, 0, 0),
            1.25,
        ),
        ("gpt-5 output", "gpt-5", usage(0, 1_000_000, 0, 0), 10.0),
        (
            "gpt-5-nano output",
            "gpt-5-nano",
            usage(0, 1_000_000, 0, 0),
            0.4,
        ),
        (
            "gpt-5.1-codex-mini output",
            "gpt-5.1-codex-mini",
            usage(0, 1_000_000, 0, 0),
            2.0,
        ),
        (
            "gpt-4.1-nano output",
            "gpt-4.1-nano",
            usage(0, 1_000_000, 0, 0),
            0.4,
        ),
        (
            "gpt-5.3-codex-spark input",
            "gpt-5.3-codex-spark",
            usage(1_000_000, 0, 0, 0),
            0.4,
        ),
        ("gpt-5.4 input", "gpt-5.4", usage(1_000_000, 0, 0, 0), 2.5),
        (
            "gpt-5.4-mini input",
            "gpt-5.4-mini",
            usage(1_000_000, 0, 0, 0),
            0.75,
        ),
        (
            "gpt-5.6-terra input",
            "gpt-5.6-terra",
            usage(1_000_000, 0, 0, 0),
            2.5,
        ),
        (
            "gpt-6-astra input",
            "gpt-6-astra",
            usage(100_000, 0, 0, 0),
            1.0,
        ),
        (
            "gpt-6-astra output",
            "openai/gpt-6-astra",
            usage(0, 100_000, 0, 0),
            5.0,
        ),
        (
            "gpt-6-astra long-context input above 272K uses doubled rates",
            "gpt-6-astra",
            usage(300_000, 10_000, 0, 0),
            300_000.0 * 20.0 / 1_000_000.0 + 10_000.0 * 75.0 / 1_000_000.0,
        ),
        (
            "gpt-5.6-luna output",
            "gpt-5.6-luna",
            usage(0, 1_000_000, 0, 0),
            6.0,
        ),
        ("gpt-5.5 output", "gpt-5.5", usage(0, 1_000_000, 0, 0), 30.0),
        (
            "unknown model has zero cost",
            "custom-model",
            usage(1_000_000, 0, 0, 0),
            0.0,
        ),
    ];
    assert_eq!(cases.len(), 21, "complete Go TestCalculateCost table");
    for (name, model, usage, want_cost) in cases {
        let estimate = cost::openai(model, &usage);
        // Go CalculateCost discards EstimateCost's known flag. Rust instead
        // exposes Option: adapt None to zero ONLY for the source unknown case.
        let got = if name == "unknown model has zero cost" {
            assert_eq!(estimate, None, "TestCalculateCost/{name}");
            estimate.unwrap_or(0.0)
        } else {
            estimate.unwrap_or_else(|| panic!("TestCalculateCost/{name}: unexpected unknown price"))
        };
        assert!(
            (got - want_cost).abs() <= 1e-9,
            "TestCalculateCost/{name}: CalculateCost({model:?}, {usage:?}) = {got:?}, want {want_cost:?}"
        );
    }
}

#[test]
fn openai_estimate_cost_original_table() {
    // (Go subtest name, model, usage, wantCost, wantKnown), in source order.
    let cases = [
        (
            "gpt-4 alias uses gpt-4.1 pricing",
            "gpt-4",
            usage(1_000_000, 0, 0, 0),
            2.0,
            true,
        ),
        (
            "whitespace trimmed",
            " gpt-4 ",
            usage(0, 1_000_000, 0, 0),
            8.0,
            true,
        ),
        (
            "openai provider prefix stripped",
            "openai/gpt-5.6-terra",
            usage(0, 1_000_000, 0, 0),
            15.0,
            true,
        ),
        (
            "gpt-5.6 alias uses sol pricing",
            "gpt-5.6",
            usage(0, 1_000_000, 0, 0),
            20.0,
            true,
        ),
        (
            "Daybreak Blue alias uses sol pricing",
            "daybreak-blue-latest",
            usage(0, 1_000_000, 0, 0),
            20.0,
            true,
        ),
        (
            "prefixed Daybreak Blue alias uses sol pricing",
            "openai/gpt-daybreak-blue-latest",
            usage(0, 1_000_000, 0, 0),
            20.0,
            true,
        ),
        (
            "Daybreak Red alias uses cyber pricing",
            "daybreak-red-latest",
            usage(0, 1_000_000, 0, 0),
            75.0,
            true,
        ),
        (
            "gpt-prefixed Daybreak Red alias uses cyber pricing",
            "gpt-daybreak-red-latest",
            usage(0, 1_000_000, 0, 0),
            75.0,
            true,
        ),
        (
            "unknown model is not known",
            "custom-model",
            usage(1_000_000, 0, 0, 0),
            0.0,
            false,
        ),
        (
            "gpt-5.6-luna cached input uses cached price",
            "gpt-5.6-luna",
            usage(1_000_000, 0, 800_000, 0),
            0.28,
            true,
        ),
        (
            "gpt-5.6-luna applies cache-write premium",
            "gpt-5.6-luna",
            usage(1_000_000, 0, 200_000, 300_000),
            0.895,
            true,
        ),
        (
            "gpt-5.6-sol uses short-context Daybreak pricing at threshold",
            "gpt-5.6-sol",
            usage(272_000, 1_000_000, 100_000, 100_000),
            20.828,
            true,
        ),
        (
            "gpt-5.6-sol uses long-context Daybreak pricing above threshold",
            "gpt-5.6-sol",
            usage(1_000_000, 1_000_000, 200_000, 300_000),
            37.16,
            true,
        ),
        (
            "gpt-5.6-cyber uses its only price tier above long-context threshold",
            "gpt-5.6-cyber",
            usage(1_000_000, 1_000_000, 200_000, 300_000),
            86.1875,
            true,
        ),
        (
            "gpt-5.3-codex applies cached discount",
            "gpt-5.3-codex",
            usage(1_000_000, 0, 800_000, 0),
            0.49,
            true,
        ),
    ];
    assert_eq!(cases.len(), 15, "complete Go TestEstimateCost table");
    for (name, model, usage, want_cost, want_known) in cases {
        // Rust Some(cost) corresponds to Go (cost, true); None corresponds
        // to (0, false). Check the known flag as well as the numeric cost,
        // so None can never silently satisfy a known-price expectation.
        let (got_cost, got_known) = match cost::openai(model, &usage) {
            Some(cost) => (cost, true),
            None => (0.0, false),
        };
        assert!(
            (got_cost - want_cost).abs() <= 1e-9 && got_known == want_known,
            "TestEstimateCost/{name}: EstimateCost({model:?}, {usage:?}) = ({got_cost:?}, {got_known}), want ({want_cost:?}, {want_known})"
        );
    }
}
