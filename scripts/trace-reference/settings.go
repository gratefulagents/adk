package main

import (
	"encoding/json"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
)

func mergedSettingsCases() []map[string]any {
	bases := []string{
		`{}`,
		`{"temperature":0.75,"max_tokens":1000,"top_p":0.9,"tool_choice":"auto","parallel_tool_calls":true,"thinking_budget":500,"reasoning_effort":"high","text_verbosity":"low","stop_sequences":["base"]}`,
		`{"temperature":0,"max_tokens":-10,"top_p":0,"tool_choice":" ","parallel_tool_calls":false,"thinking_budget":-2,"reasoning_effort":"\t","text_verbosity":" ","stop_sequences":[""]}`,
	}
	overrides := []string{
		`{}`,
		`{"temperature":null,"max_tokens":null,"top_p":null,"tool_choice":null,"parallel_tool_calls":null,"thinking_budget":null,"reasoning_effort":null,"text_verbosity":null,"stop_sequences":null}`,
		`{"temperature":0,"max_tokens":0,"top_p":0,"tool_choice":"","parallel_tool_calls":false,"thinking_budget":0,"reasoning_effort":"","text_verbosity":"","stop_sequences":[]}`,
		`{"temperature":-0.5,"max_tokens":-1,"top_p":-0.1,"thinking_budget":-1}`,
		`{"temperature":0.25,"max_tokens":20,"top_p":0.2,"tool_choice":"none","parallel_tool_calls":false,"thinking_budget":10,"reasoning_effort":"low","text_verbosity":"high","stop_sequences":["override"]}`,
		`{"temperature":0}`,
		`{"parallel_tool_calls":false}`,
		`{"top_p":0}`,
		`{"max_tokens":1,"thinking_budget":1}`,
		`{"tool_choice":" ","reasoning_effort":"\t","text_verbosity":" "}`,
		`{"stop_sequences":["","new"]}`,
		`{"stop_sequences":[null]}`,
	}
	cases := []map[string]any{}
	for _, baseJSON := range bases {
		for _, overrideJSON := range overrides {
			var base, override agent.ModelSettings
			must(json.Unmarshal([]byte(baseJSON), &base))
			must(json.Unmarshal([]byte(overrideJSON), &override))
			encoded, err := json.Marshal(base.Merge(override))
			must(err)
			cases = append(cases, map[string]any{"base": baseJSON, "override": overrideJSON, "merged": string(encoded)})
		}
	}
	return cases
}

func routingSettingsCases() map[string]any {
	var labels []any
	for _, reasoning := range []string{"", "none", "minimal", "low", "medium", "high", "xhigh", "max", " HİGH ", "hi\u0307gh", "invalid", "\u00a0MEDIUM\u0085"} {
		for _, verbosity := range []string{"", "low", "medium", "high", " HİGH ", "invalid"} {
			labels = append(labels, map[string]any{
				"reasoning": reasoning, "verbosity": verbosity,
				"settings": agent.ModeRoutingSettings(reasoning, verbosity),
			})
		}
	}
	defaults := map[string]any{}
	for name, config := range map[string]runtime.Config{
		"default": {},
		"blank":   {Reasoning: "  ", Verbosity: "  "},
		"none":    {Reasoning: "none", Verbosity: "low"},
		"max":     {Reasoning: "max", Verbosity: "high"},
		"invalid": {Reasoning: "invalid", Verbosity: "invalid"},
	} {
		a, _ := runtime.BuildAgent(config, nil, runtime.ToolBundle{})
		defaults[name] = a.ModelSettings
	}
	return map[string]any{"labels": labels, "builder_defaults": defaults}
}
