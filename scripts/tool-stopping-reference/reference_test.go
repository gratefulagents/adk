// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func TestNativeToolStoppingReference(t *testing.T) {
	cases := []map[string]any{}
	for _, name := range []string{"continue", "stop_all", "named", "case_miss", "space_miss", "first", "second_named", "object", "string", "null", "false", "disabled", "schema", "guardrail", "pause", "handoff", "steering"} {
		for _, streamed := range []bool{false, true} {
			guards := []any{}
			read := &FunctionTool{ToolName: "read", ReadOnly: true, Schema: json.RawMessage(`{}`), Fn: func(context.Context, json.RawMessage) (string, error) { return "read-output", nil }}
			other := &FunctionTool{ToolName: "other", ReadOnly: true, Schema: json.RawMessage(`{}`), Fn: func(context.Context, json.RawMessage) (string, error) { return "other-output", nil }}
			a := &Agent{Name: "agent", Tools: []Tool{read, other}, StopAtTools: &StopAtTools{ToolNames: []string{"read"}}, OutputGuardrails: []OutputGuardrail{{Name: "check", Fn: func(_ *RunContext, _ *Agent, output any) (*GuardrailResult, error) {
				guards = append(guards, output)
				return &GuardrailResult{TripwireTriggered: name == "guardrail"}, nil
			}}}}
			switch name {
			case "continue":
				a.StopAtTools = nil
			case "stop_all":
				a.StopAtTools = nil
				a.ToolUseBehavior = StopOnFirstTool
			case "case_miss":
				a.StopAtTools.ToolNames = []string{"READ"}
			case "space_miss":
				a.StopAtTools.ToolNames = []string{" read "}
			case "second_named":
				a.StopAtTools.ToolNames = []string{"other"}
			}
			if name == "first" || name == "second_named" || name == "schema" || name == "continue" || name == "steering" {
				a.ToolsToFinalOutput = &ToolsToFinalOutputResult{IsFinalOutput: true}
			}
			if name == "schema" {
				a.OutputType = &OutputSchema{Schema: json.RawMessage(`{"type":"object"}`)}
			}
			if raw, ok := map[string]string{"object": `{"done":true}`, "string": `"quoted"`, "null": `null`, "false": `false`, "disabled": `{"ignored":true}`}[name]; ok {
				a.ToolsToFinalOutput = &ToolsToFinalOutputResult{IsFinalOutput: name != "disabled", Output: json.RawMessage(raw)}
			}
			items := []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "a", Name: "read", Input: json.RawMessage(`{}`)}}}
			if name == "second_named" {
				items = append(items, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "b", Name: "other", Input: json.RawMessage(`{}`)}})
			}
			cfg := RunConfig{MaxTurns: 3}
			if name == "pause" {
				read.ToolName = "present_plan"
				items[0].ToolCall.Name = "present_plan"
				a.StopAtTools.ToolNames = []string{"present_plan"}
			}
			if name == "handoff" {
				a.Handoffs = []*Handoff{NewHandoff(&Agent{Name: "child"})}
				items[0].ToolCall.Name = "transfer_to_child"
				a.StopAtTools.ToolNames = []string{"transfer_to_child"}
			}
			if name == "steering" {
				count := 0
				cfg.ImmediateInputFinalizer = func(context.Context) ([]RunItem, error) {
					count++
					if count == 1 {
						return []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "steering"}}}, nil
					}
					return nil, nil
				}
			}
			model := &subagentToolMockModel{responses: []*ModelResponse{{Items: items}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}
			runner := NewRunnerWithModel(model)
			var result *RunResult
			var err error
			if streamed {
				s := runner.RunStreamed(context.Background(), a, nil, cfg)
				for range s.Events {
				}
				result = s.FinalResult()
				err = s.Err()
			} else {
				result, err = runner.Run(context.Background(), a, nil, cfg)
			}
			var output any
			var text string
			if result != nil {
				output = result.FinalOutput
				text = result.FinalText()
			}
			cases = append(cases, map[string]any{"name": name, "streamed": streamed, "error": err != nil, "output": output, "text": text, "guards": guards, "requests": len(model.requests)})
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
