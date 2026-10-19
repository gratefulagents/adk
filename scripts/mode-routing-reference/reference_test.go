// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"fmt"
	"os"
	"testing"
)

func TestNativeModeRoutingReference(t *testing.T) {
	type input struct {
		BaseModel string            `json:"base_model"`
		Provider  string            `json:"provider"`
		Routing   *ModeModelRouting `json:"routing"`
	}
	inputs := []input{}
	providers := []string{"", " OpenAI ", " ANTHROPIC ", "copilot", "claude", "anthropİc", "openai-compatible"}
	for _, provider := range providers {
		for _, model := range []string{"", " small ", "MEDIUM", "Large", " Custom/猫 "} {
			inputs = append(inputs, input{model, provider, nil}, input{model, provider, &ModeModelRouting{}})
		}
	}
	routes := []*ModeModelRouting{}
	for _, model := range []string{"", " \t ", " small ", "MEDIUM", "Large", " Custom/猫 "} {
		routes = append(routes, &ModeModelRouting{DefaultModel: model, FallbackModels: []string{" small ", "", "medium", "small", " RAW/猫 "}, ReasoningLevel: "high", TextVerbosity: "low"})
	}
	for _, level := range []string{"", " NONE ", "minimal", "LOW", "medium", " HİGH ", "xhigh", "max", "invalid"} {
		routes = append(routes, &ModeModelRouting{ReasoningLevel: level, TextVerbosity: " MEDİUM ", RoleOverrides: map[string]ModeRoleModelRouting{
			"worker":   {ReasoningLevel: "none", TextVerbosity: "high"},
			"Worker":   {ReasoningLevel: "invalid", TextVerbosity: "invalid"},
			" worker ": {ReasoningLevel: "max", TextVerbosity: " low "},
		}})
		routes = append(routes, &ModeModelRouting{ReasoningLevel: "high", TextVerbosity: "low", RoleOverrides: map[string]ModeRoleModelRouting{"worker": {ReasoningLevel: level, TextVerbosity: level}}})
	}
	for _, fallbacks := range [][]string{nil, {}, {"", " \t "}, {" small ", " RAW ", "small", ""}} {
		routes = append(routes, &ModeModelRouting{FallbackModels: fallbacks})
		routes = append(routes, &ModeModelRouting{DefaultModel: "medium", FallbackModels: []string{"large"}, RoleOverrides: map[string]ModeRoleModelRouting{"worker": {Model: " LARGE ", FallbackModels: fallbacks}, "Worker": {Model: " \t ", FallbackModels: fallbacks}, " worker ": {Model: " Raw ", FallbackModels: fallbacks}}})
	}
	for _, provider := range providers {
		for _, routing := range routes {
			inputs = append(inputs, input{" small ", provider, routing})
		}
	}
	cases := []any{}
	for index, in := range inputs {
		before, err := json.Marshal(in)
		if err != nil {
			t.Fatal(err)
		}
		encode := func(value any) string {
			encoded, err := json.Marshal(value)
			if err != nil {
				t.Fatal(err)
			}
			return string(encoded)
		}
		top := ResolveModeRouting(in.BaseModel, in.Provider, in.Routing)
		roles := []any{}
		for _, name := range []string{"worker", "Worker", " worker ", "worker ", "absent"} {
			result := ResolveRoleModeRouting(in.BaseModel, in.Provider, name, in.Routing)
			roles = append(roles, map[string]any{"name": name, "result": result, "output_json": encode(result)})
		}
		if string(before) != encode(in) {
			t.Fatal("routing resolution mutated its input")
		}
		cases = append(cases, map[string]any{"id": fmt.Sprintf("routing-%d", index), "input": in, "resolved_model": ResolveModelForProvider(in.BaseModel, in.Provider), "top": top, "top_json": encode(top), "roles": roles})
	}
	encoded, err := json.Marshal(map[string]any{"schema_version": 1, "cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("MODE_ROUTING_OUTPUT"), encoded, 0600); err != nil {
		t.Fatal(err)
	}
}
