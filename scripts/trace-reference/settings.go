package main

import (
	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
)

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
