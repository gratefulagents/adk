// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"testing"
)

type costReferenceResponse struct {
	Usage     Usage   `json:"usage"`
	CostUSD   float64 `json:"cost_usd"`
	CostKnown bool    `json:"cost_known"`
}
type costReferenceCase struct {
	Name           string                  `json:"name"`
	Usage          Usage                   `json:"usage"`
	Responses      []costReferenceResponse `json:"responses"`
	FallbackKind   string                  `json:"fallback_kind"`
	FallbackCost   float64                 `json:"fallback_cost"`
	CostUSD        float64                 `json:"cost_usd"`
	CostKnown      bool                    `json:"cost_known"`
	FallbackUsages []Usage                 `json:"fallback_usages"`
}

// Embedding Model satisfies irrelevant generation methods; this oracle must never
// invoke them. Legacy deliberately has no EstimateCost method.
type costReferenceLegacy struct {
	Model
	amount float64
	calls  *[]Usage
}

func (m *costReferenceLegacy) CalculateCost(u Usage) float64 {
	*m.calls = append(*m.calls, u)
	return m.amount
}

type costReferenceEstimator struct {
	costReferenceLegacy
	known bool
}

func (m *costReferenceEstimator) EstimateCost(u Usage) (float64, bool) {
	return m.CalculateCost(u), m.known
}
func TestNativeResultCostReference(t *testing.T) {
	response := costReferenceResponse{Usage: Usage{Requests: 1, InputTokens: 7, OutputTokens: 4, CacheReadTokens: 2, CacheCreateTokens: 3}, CostUSD: .75, CostKnown: true}
	same := response.Usage
	residual := Usage{Requests: 3, InputTokens: 10, OutputTokens: 5, CacheReadTokens: 8, CacheCreateTokens: 6}
	unknown := response
	unknown.CostKnown = false
	cases := []costReferenceCase{
		{Name: "response_sum", Usage: Usage{Requests: 2}, Responses: []costReferenceResponse{{CostUSD: .25, CostKnown: true}, {CostUSD: .75, CostKnown: true}}, FallbackKind: "known", FallbackCost: 99},
		{Name: "unknown_response", Responses: []costReferenceResponse{{CostUSD: .25, CostKnown: true}, {CostUSD: 0, CostKnown: false}}, FallbackKind: "known", FallbackCost: 99},
		{Name: "empty_known", Usage: residual, FallbackKind: "known", FallbackCost: .75},
		{Name: "empty_unknown_nonzero", Usage: residual, FallbackKind: "unknown", FallbackCost: .25},
		{Name: "empty_no_model", Usage: residual, FallbackKind: "nil"},
		{Name: "empty_zero_known", FallbackKind: "known"},
		{Name: "empty_zero_unknown", FallbackKind: "unknown"},
		{Name: "residual_known", Usage: residual, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: 1},
		{Name: "residual_unknown_nonzero", Usage: residual, Responses: []costReferenceResponse{response}, FallbackKind: "unknown", FallbackCost: .25},
		{Name: "residual_no_model", Usage: residual, Responses: []costReferenceResponse{response}, FallbackKind: "nil"},
		{Name: "requests_only", Usage: Usage{Requests: 3, InputTokens: 7, OutputTokens: 4, CacheReadTokens: 2, CacheCreateTokens: 3}, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: 99},
		{Name: "cache_read_only", Usage: Usage{Requests: 1, InputTokens: 7, OutputTokens: 4, CacheReadTokens: 3, CacheCreateTokens: 3}, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: .25},
		{Name: "cache_create_only", Usage: Usage{Requests: 1, InputTokens: 7, OutputTokens: 4, CacheReadTokens: 2, CacheCreateTokens: 4}, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: .25},
		{Name: "componentwise_clamp", Usage: Usage{InputTokens: 2, OutputTokens: 9, CacheReadTokens: 1, CacheCreateTokens: 2}, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: .25},
		{Name: "all_overstated", Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: 99},
		{Name: "legacy_empty", Usage: residual, FallbackKind: "legacy", FallbackCost: .25},
		{Name: "legacy_residual", Usage: residual, Responses: []costReferenceResponse{response}, FallbackKind: "legacy", FallbackCost: .25},
		{Name: "unknown_response_known_residual", Usage: residual, Responses: []costReferenceResponse{unknown}, FallbackKind: "known", FallbackCost: 1},
		{Name: "aggregate_residual_once", Usage: Usage{Requests: 3, InputTokens: 17, OutputTokens: 4, CacheReadTokens: 2, CacheCreateTokens: 3}, Responses: []costReferenceResponse{response}, FallbackKind: "known", FallbackCost: .25},
		{Name: "zero_priced_residual", Usage: residual, Responses: []costReferenceResponse{response}, FallbackKind: "known"},
		{Name: "exact_usage_no_fallback", Usage: same, Responses: []costReferenceResponse{response}, FallbackKind: "unknown", FallbackCost: 99},
	}
	for i := range cases {
		c := &cases[i]
		c.FallbackUsages = []Usage{}
		result := &RunResult{Usage: c.Usage}
		for _, r := range c.Responses {
			result.RawResponses = append(result.RawResponses, ModelResponse{Usage: r.Usage, CostUSD: r.CostUSD, CostKnown: r.CostKnown})
		}
		var model Model
		base := costReferenceLegacy{amount: c.FallbackCost, calls: &c.FallbackUsages}
		switch c.FallbackKind {
		case "known", "unknown":
			model = &costReferenceEstimator{costReferenceLegacy: base, known: c.FallbackKind == "known"}
		case "legacy":
			model = &base
		case "nil":
		default:
			t.Fatal("unknown model kind")
		}
		c.CostUSD, c.CostKnown = estimateRunResultCost(result, model)
	}
	floatCases := []map[string]string{}
	for _, value := range []float64{0, math.Copysign(0, -1), 1, .25, -1.25, 1e-6, 1e-7, 1e20, 1e21, math.MaxFloat64, math.SmallestNonzeroFloat64} {
		encoded, err := json.Marshal(ModelResponse{CostUSD: value})
		if err != nil {
			t.Fatal(err)
		}
		var fields map[string]json.RawMessage
		if err := json.Unmarshal(encoded, &fields); err != nil {
			t.Fatal(err)
		}
		floatCases = append(floatCases, map[string]string{"bits": fmt.Sprintf("%016x", math.Float64bits(value)), "json": string(fields["CostUSD"])})
	}
	data, err := json.Marshal(map[string]any{"cases": cases, "float_cases": floatCases, "compat_cases": costCompatibilityCases(t)})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("COST_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}

func costCompatibilityCases(t *testing.T) []map[string]any {
	totals := []Usage{{}, {Requests: 3, InputTokens: 10, OutputTokens: 5, CacheReadTokens: 8, CacheCreateTokens: 6}, {Requests: -1, InputTokens: -7, OutputTokens: -3, CacheReadTokens: -2, CacheCreateTokens: -8}, {Requests: math.MaxInt64, InputTokens: math.MaxInt64, OutputTokens: 1, CacheReadTokens: math.MinInt64, CacheCreateTokens: math.MaxInt64}, {Requests: math.MinInt64, InputTokens: math.MinInt64, OutputTokens: math.MaxInt64, CacheReadTokens: 1}}
	amounts := []float64{0, math.Copysign(0, -1), .25, -1, math.NaN(), math.Inf(1), math.Inf(-1), math.MaxFloat64}
	var cases []map[string]any
	for totalIndex, total := range totals {
		for responseKind := 0; responseKind < 5; responseKind++ {
			for _, kind := range []string{"nil", "legacy", "known", "unknown"} {
				for amountIndex, amount := range amounts {
					for _, present := range []bool{false, true} {
						responses := []ModelResponse{}
						switch responseKind {
						case 1:
							responses = []ModelResponse{{Usage: Usage{Requests: 1, InputTokens: 7, OutputTokens: 4, CacheReadTokens: 2, CacheCreateTokens: 3}, CostUSD: amount, CostKnown: true}}
						case 2:
							responses = []ModelResponse{{Usage: total, CostUSD: amount, CostKnown: true}, {Usage: total, CostUSD: .25, CostKnown: false}}
						case 3:
							responses = []ModelResponse{{Usage: Usage{Requests: -1, InputTokens: -7, OutputTokens: -3, CacheReadTokens: -2, CacheCreateTokens: -8}, CostUSD: amount, CostKnown: false}}
						case 4:
							responses = []ModelResponse{{Usage: Usage{Requests: math.MaxInt64, InputTokens: math.MaxInt64, OutputTokens: math.MaxInt64, CacheReadTokens: math.MaxInt64, CacheCreateTokens: math.MaxInt64}, CostUSD: amount, CostKnown: true}, {Usage: Usage{Requests: 1, InputTokens: 1, OutputTokens: 1, CacheReadTokens: 1, CacheCreateTokens: 1}, CostUSD: amount, CostKnown: true}}
						}
						calls := []Usage{}
						base := costReferenceLegacy{amount: amount, calls: &calls}
						var model Model
						switch kind {
						case "legacy":
							model = &base
						case "known", "unknown":
							model = &costReferenceEstimator{costReferenceLegacy: base, known: kind == "known"}
						}
						var result *RunResult
						if present {
							result = &RunResult{Usage: total, RawResponses: responses}
						}
						cost, known := estimateRunResultCost(result, model)
						observedResponses := []map[string]any{}
						for _, response := range responses {
							observedResponses = append(observedResponses, map[string]any{"usage": response.Usage, "cost_bits": fmt.Sprintf("%016x", math.Float64bits(response.CostUSD)), "known": response.CostKnown})
						}
						cases = append(cases, map[string]any{"id": fmt.Sprintf("%d/%d/%s/%d/%t", totalIndex, responseKind, kind, amountIndex, present), "present": present, "usage": total, "responses": observedResponses, "fallback_kind": kind, "fallback_bits": fmt.Sprintf("%016x", math.Float64bits(amount)), "cost_bits": fmt.Sprintf("%016x", math.Float64bits(cost)), "known": known, "calls": calls})
					}
				}
			}
		}
	}
	if len(cases) != 1600 {
		t.Fatalf("unexpected compatibility cases: %d", len(cases))
	}
	return cases
}
