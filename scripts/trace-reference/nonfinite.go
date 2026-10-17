package main

import (
	"math"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tracestore"
)

func nonFiniteSpanCases(store tracestore.TraceStore) map[string]any {
	cases := map[string]any{}
	for label, cost := range map[string]float64{"nan": math.NaN(), "positive": math.Inf(1), "negative": math.Inf(-1)} {
		for kind, data := range map[string]agent.SpanData{
			"session":    agent.SessionSpanData{CostUSD: cost},
			"generation": agent.GenerationSpanData{CostUSD: cost},
			"subagent":   agent.SubagentSpanData{CostUSD: cost},
		} {
			name := label + "-" + kind
			writer := tracestore.NewTraceWriter(store)
			must(writer.InitRun(tracestore.RunMetadata{RunID: name}))
			span := agent.NewSpan(kind, "", data)
			writer.OnSpanStart(span)
			span.Finish()
			writer.OnSpanEnd(span)
			cases[name] = writer.Health()
		}
	}
	return cases
}
