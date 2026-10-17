// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"os"
	"testing"

	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

func TestNativeSpecialistCatalogReference(t *testing.T) {
	type input struct {
		Name           string
		Catalog        RoleCatalog
		BaseModel      string
		FallbackModels []string
		Provider       string
		BaseSettings   ModelSettings
		Routing        *sdkmode.ModelRouting
		NoRunner       bool
		NoTools        bool
	}
	roles := RoleCatalog{
		{Name: " \t\u0085"},
		{Name: " worker ", Description: "desc", ModelOverride: " small ", FallbackModels: []string{" raw ", ""}, ToolAccess: "read-only"},
		{Name: "worker", Instructions: "ignored duplicate", ModelOverride: "large"},
		{Name: "Worker", ToolAccess: "full"},
		{Name: "a.b", ToolAccess: "typo"}, {Name: "a b", ToolAccess: "read-only"}, {Name: "---界---", ToolAccess: "read-only"},
	}
	inputs := []input{
		{Name: "raw-and-selection", Catalog: roles, BaseModel: " base ", FallbackModels: []string{" base-fallback ", ""}},
		{Name: "missing-runner", Catalog: roles, NoRunner: true},
		{Name: "empty-catalog", Catalog: RoleCatalog{}},
		{Name: "empty-tool-set", Catalog: roles, NoTools: true},
		{Name: "none-without-base-budget", Catalog: RoleCatalog{{Name: "worker"}}, Routing: &sdkmode.ModelRouting{ReasoningLevel: "high", RoleOverrides: map[string]sdkmode.RoleModelRouting{"worker": {ReasoningLevel: "none"}}}},
		{Name: "blank-mode-fallback-retains-base", Catalog: RoleCatalog{{Name: "worker", ModelOverride: " ", FallbackModels: []string{}}}, BaseModel: " base ", FallbackModels: []string{" raw-base ", ""}, Routing: &sdkmode.ModelRouting{FallbackModels: []string{" "}, RoleOverrides: map[string]sdkmode.RoleModelRouting{"worker": {FallbackModels: []string{" "}}}}},
	}
	for _, provider := range []string{"", "openai", " ANTHROPIC ", "openrouter", "unknown", "ANTHROPİC"} {
		for _, model := range []string{"", " small ", "medium", "large", " vendor/model ", "medİum"} {
			inputs = append(inputs, input{Name: "empty-routing/" + provider + "/" + model, Catalog: RoleCatalog{{Name: "worker"}}, BaseModel: model, Provider: provider, Routing: &sdkmode.ModelRouting{}})
		}
	}
	for _, fallback := range [][]string{nil, {}, {" ", "\t"}, {" medium ", "", " large "}} {
		for _, reasoning := range []string{"none", "low", "invalid"} {
			inputs = append(inputs, input{Name: "routing-precedence/" + reasoning, Catalog: roles, BaseModel: "base", Provider: "anthropic", FallbackModels: []string{"base-fb"}, BaseSettings: ModelSettings{ThinkingBudget: 777, ReasoningEffort: "base", TextVerbosity: "base", ToolChoice: "auto"}, Routing: &sdkmode.ModelRouting{DefaultModel: " large ", FallbackModels: []string{"small"}, ReasoningLevel: "high", TextVerbosity: "high", RoleOverrides: map[string]sdkmode.RoleModelRouting{"worker": {Model: " medium ", FallbackModels: fallback, ReasoningLevel: reasoning, TextVerbosity: "low"}}}})
		}
	}
	cases := []any{}
	for _, in := range inputs {
		calls := []string{}
		read := &FunctionTool{ToolName: "read", ReadOnly: true}
		write := &FunctionTool{ToolName: "write"}
		signal := &FunctionTool{ToolName: "finish", ReadOnly: true}
		renamed := &nativeAccessAdapter{Tool: &FunctionTool{ToolName: "rename"}, calls: &calls, id: "rename", replacement: signal}
		tools := []Tool{read, write, renamed, read, signal}
		if in.NoTools {
			tools = nil
		}
		project := func(agent *Agent) any {
			names := []string{}
			for _, tool := range agent.Tools {
				names = append(names, tool.Name())
			}
			fb := append([]string{}, agent.FallbackModels...)
			return map[string]any{"name": agent.Name, "instructions": agent.Instructions, "description": agent.HandoffDescription, "model": agent.Model, "fallbacks": fb, "settings": agent.ModelSettings, "tools": names}
		}
		raw := []any{}
		for _, agent := range BuildSpecialistsFromCatalog(in.Catalog) {
			raw = append(raw, project(agent))
		}
		var runner *Runner
		if !in.NoRunner {
			runner = &Runner{}
		}
		var mode *sdkmode.TemplateSpec
		if in.Routing != nil {
			mode = &sdkmode.TemplateSpec{ModelRouting: in.Routing}
		}
		result := BuildSpecialistToolsFromCatalog(in.Catalog, SpecialistBuildOptions{Runner: runner, Tools: tools, BaseModel: in.BaseModel, FallbackModels: in.FallbackModels, Provider: in.Provider, BaseModelSettings: in.BaseSettings, ModeSnapshot: mode})
		agents := map[string]any{}
		for name, agent := range result.Agents {
			agents[name] = project(agent)
			if len(agent.Tools) > 0 && agent.Tools[0] != read {
				t.Fatal("lost shared tool identity")
			}
		}
		wrappers := []any{}
		for _, tool := range result.Tools {
			wrappers = append(wrappers, map[string]any{"name": tool.Name(), "description": tool.Description(), "readOnly": tool.IsReadOnly(), "approval": tool.NeedsApproval()})
		}
		cases = append(cases, map[string]any{"input": in, "raw": raw, "agents": agents, "wrappers": wrappers, "adapterCalls": calls})
	}
	data, err := json.Marshal(map[string]any{"cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("CATALOG_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
