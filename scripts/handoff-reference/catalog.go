package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
	sdkruntime "github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/shell"
)

type catalogInput struct {
	AgentName             string                      `json:"agent_name"`
	Instructions          string                      `json:"instructions"`
	Model                 string                      `json:"model"`
	FallbackModels        []string                    `json:"fallback_models"`
	Reasoning             string                      `json:"reasoning"`
	Verbosity             string                      `json:"verbosity"`
	MaxTokens             int                         `json:"max_tokens"`
	ModelSettings         *agentsdk.ModelSettings     `json:"model_settings"`
	Roles                 agentsdk.RoleCatalog        `json:"roles"`
	Mode                  *sdkmode.TemplateSpec       `json:"mode"`
	Handoffs              sdkruntime.HandoffFeatures  `json:"handoffs"`
	SubAgents             sdkruntime.SubAgentFeatures `json:"subagents"`
	ParentTools           bool                        `json:"parent_tools"`
	ModeRouting           bool                        `json:"mode_routing"`
	ParallelToolCalls     bool                        `json:"parallel_tool_calls"`
	LegacyEnableHandoffs  bool                        `json:"legacy_enable_handoffs"`
	LegacyEnableSubAgents bool                        `json:"legacy_enable_subagents"`
	HostTools             []toolView                  `json:"host_tools"`
	MCPServers            []string                    `json:"mcp_servers"`
}

type catalogScenario struct {
	Name       string       `json:"name"`
	SourceOnly []string     `json:"source_only"`
	Input      catalogInput `json:"input"`
}

type toolView struct {
	Name           string `json:"name"`
	ReadOnly       bool   `json:"read_only"`
	Implementation string `json:"implementation"`
}

type agentView struct {
	Name                string                 `json:"name"`
	Instructions        string                 `json:"instructions"`
	DynamicInstructions bool                   `json:"dynamic_instructions"`
	HandoffDescription  string                 `json:"handoff_description"`
	Model               string                 `json:"model"`
	FallbackModels      []string               `json:"fallback_models"`
	Settings            agentsdk.ModelSettings `json:"model_settings"`
	Tools               []toolView             `json:"tools"`
	MCPServers          []string               `json:"mcp_servers"`
}

type handoffView struct {
	ToolName            string        `json:"tool_name"`
	Description         string        `json:"description"`
	Target              agentView     `json:"target"`
	SpecialistKey       *string       `json:"specialist_key"`
	HasInputFilter      bool          `json:"has_input_filter"`
	FilterOutput        []encodedItem `json:"filter_output"`
	HasInputType        bool          `json:"has_input_type"`
	HasOnHandoff        bool          `json:"has_on_handoff"`
	HasEnabledPredicate bool          `json:"has_enabled_predicate"`
	Tool                toolView      `json:"tool"`
}

type catalogOutput struct {
	Parent        agentView            `json:"parent"`
	ReturnedTools []toolView           `json:"returned_tools"`
	Specialists   map[string]agentView `json:"specialists"`
	Handoffs      []handoffView        `json:"handoffs"`
}

type catalogObservation struct {
	catalogScenario
	Output catalogOutput `json:"output"`
}

type catalogFixture struct {
	SchemaVersion int                  `json:"schema_version"`
	SDKRevision   string               `json:"sdk_revision"`
	Scope         string               `json:"scope"`
	FilterProbe   []encodedItem        `json:"filter_probe"`
	Cases         []catalogObservation `json:"cases"`
}

// Any accidental model execution fails rather than resolving credentials or networking.
type offlineModel struct{}

func (offlineModel) GetResponse(context.Context, agentsdk.ModelRequest) (*agentsdk.ModelResponse, error) {
	panic("composition oracle must not call a model")
}
func (offlineModel) StreamResponse(context.Context, agentsdk.ModelRequest) (*agentsdk.ModelStream, error) {
	panic("composition oracle must not stream a model")
}
func (offlineModel) GetRetryAdvice(error) *agentsdk.ModelRetryAdvice {
	panic("composition oracle must not retry")
}
func (offlineModel) CalculateCost(agentsdk.Usage) float64 {
	panic("composition oracle must not calculate cost")
}
func (offlineModel) Provider() string { return "offline" }

type observationTool struct {
	name     string
	readOnly bool
}

func (t observationTool) Name() string               { return t.name }
func (t observationTool) IsReadOnly() bool           { return t.readOnly }
func (observationTool) Description() string          { return "Inert composition-only tool" }
func (observationTool) InputSchema() json.RawMessage { return json.RawMessage(`{"type":"object"}`) }
func (observationTool) Execute(context.Context, json.RawMessage, string) (agentsdk.ToolResult, error) {
	panic("composition oracle must not execute tools")
}
func (observationTool) IsEnabled(*agentsdk.RunContext) bool { return true }
func (observationTool) NeedsApproval() bool                 { return false }
func (observationTool) TimeoutSeconds() int                 { return 0 }

func projectTool(t agentsdk.Tool) toolView {
	implementation := "inert_host_tool"
	switch t.(type) {
	case *shell.BashTool:
		implementation = "sdk_bash"
	case *shell.ReadOnlyBashTool:
		implementation = "sdk_read_only_bash"
	case observationTool:
	default:
		implementation = "sdk_handoff_tool"
	}
	return toolView{t.Name(), t.IsReadOnly(), implementation}
}

func projectTools(tools []agentsdk.Tool) []toolView {
	if tools == nil {
		return nil
	}
	out := make([]toolView, len(tools))
	for i, tool := range tools {
		out[i] = projectTool(tool)
	}
	return out
}

func projectAgent(a *agentsdk.Agent) agentView {
	return agentView{a.Name, a.GetInstructions(nil), a.InstructionsFn != nil, a.HandoffDescription, a.Model, a.FallbackModels, a.ModelSettings, projectTools(a.Tools), a.MCPServers}
}

func catalogFilterProbe() []agentsdk.RunItem {
	return []agentsdk.RunItem{
		{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "retained catalog history"}},
		{Type: agentsdk.RunItemToolCall},
	}
}

func observeCatalog(in catalogInput) catalogOutput {
	tools := make([]agentsdk.Tool, 0, len(in.HostTools))
	for _, spec := range in.HostTools {
		switch spec.Implementation {
		case "sdk_bash":
			tools = append(tools, &shell.BashTool{})
		case "inert_host_tool":
			tools = append(tools, observationTool{spec.Name, spec.ReadOnly})
		default:
			panic(fmt.Sprintf("unknown input tool implementation %q", spec.Implementation))
		}
	}
	features := &sdkruntime.Features{
		Handoffs: in.Handoffs, SubAgents: in.SubAgents,
		Tools:   sdkruntime.ToolFeatures{ExtraTools: in.ParentTools},
		Modes:   sdkruntime.ModeFeatures{ModelRouting: in.ModeRouting},
		Runtime: sdkruntime.RuntimeFeatures{ParallelToolCalls: in.ParallelToolCalls},
	}
	cfg := sdkruntime.Config{
		Provider: "openai", AgentName: in.AgentName, Instructions: in.Instructions,
		Model: in.Model, FallbackModels: in.FallbackModels, ModelSettings: in.ModelSettings,
		Reasoning: in.Reasoning, Verbosity: in.Verbosity, MaxTokens: in.MaxTokens,
		RoleCatalog: in.Roles, ModeSnapshot: in.Mode, Features: features,
		EnableHandoffs: in.LegacyEnableHandoffs, EnableSubAgents: in.LegacyEnableSubAgents,
	}
	runner := agentsdk.NewRunnerWithModel(offlineModel{})
	parent, returnedTools, specialists := sdkruntime.BuildAgentWithSpecialists(cfg, runner, sdkruntime.ToolBundle{Tools: tools, MCPServers: in.MCPServers})
	out := catalogOutput{Parent: projectAgent(parent), ReturnedTools: projectTools(returnedTools)}
	if specialists != nil {
		out.Specialists = make(map[string]agentView, len(specialists))
		for key, a := range specialists {
			out.Specialists[key] = projectAgent(a)
		}
	}
	if parent.Handoffs != nil {
		out.Handoffs = make([]handoffView, len(parent.Handoffs))
		for i, h := range parent.Handoffs {
			v := handoffView{ToolName: h.ToolName, Description: h.Description, Target: projectAgent(h.Agent), HasInputFilter: h.InputFilter != nil, HasInputType: h.InputType != nil, HasOnHandoff: h.OnHandoff != nil, HasEnabledPredicate: h.IsEnabledFn != nil, Tool: projectTool(h.ToTool())}
			for key, a := range specialists {
				if h.Agent == a {
					v.SpecialistKey = &key
				}
			}
			if h.InputFilter != nil {
				v.FilterOutput = encodeItems(h.InputFilter(catalogFilterProbe(), nil))
			}
			out.Handoffs[i] = v
		}
	}
	return out
}

func generateCatalog() ([]byte, error) {
	f := catalogFixture{SchemaVersion: 1, SDKRevision: sdkRevision, Scope: "runtime.BuildAgentWithSpecialists public-definition projection; explicit features, inert injected runner/model and host tools, real SDK Bash access adapter; no model/tool execution, provider resolution, full Agent serialization or Rust outputs", FilterProbe: encodeItems(catalogFilterProbe())}
	for _, s := range catalogScenarios() {
		f.Cases = append(f.Cases, catalogObservation{s, observeCatalog(s.Input)})
	}
	b, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
