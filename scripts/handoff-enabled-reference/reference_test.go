// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

type enabledReferenceModel struct {
	*subagentToolMockModel
	after func()
}

func (m *enabledReferenceModel) GetResponse(ctx context.Context, req ModelRequest) (*ModelResponse, error) {
	m.after()
	return m.subagentToolMockModel.GetResponse(ctx, req)
}
func (m *enabledReferenceModel) StreamResponse(ctx context.Context, req ModelRequest) (*ModelStream, error) {
	m.after()
	return m.subagentToolMockModel.StreamResponse(ctx, req)
}
func TestNativeHandoffEnabledReference(t *testing.T) {
	cases := []map[string]any{}
	for _, streamed := range []bool{false, true} {
		for _, name := range []string{"default", "enabled", "disabled", "disable_after_request", "enable_after_request", "enabled_sibling"} {
			active := name != "disabled" && name != "enable_after_request" && name != "enabled_sibling"
			initial := active
			after := name != "disabled" && name != "disable_after_request" && name != "enabled_sibling"
			target := &Agent{Name: "expert", Model: "expert-model"}
			h := NewHandoff(target)
			count := 0
			h.OnHandoff = func(*RunContext, json.RawMessage) { count++ }
			if name != "default" {
				h.IsEnabledFn = func(*RunContext) bool { return active }
			}
			source := &Agent{Name: "router", Model: "router-model", Handoffs: []*Handoff{h}}
			calls := []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: h.ToolName, Input: json.RawMessage(`{}`)}}}
			if name == "enabled_sibling" {
				source.Handoffs = append(source.Handoffs, NewHandoff(&Agent{Name: "other", Model: "other-model"}))
				calls = append(calls, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h2", Name: "transfer_to_other", Input: json.RawMessage(`{}`)}})
			}
			model := &enabledReferenceModel{subagentToolMockModel: &subagentToolMockModel{responses: []*ModelResponse{{Items: calls}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}, after: func() { active = after }}
			runner := NewRunnerWithModel(model)
			var result *RunResult
			var err error
			if streamed {
				s := runner.RunStreamed(context.Background(), source, nil, RunConfig{MaxTurns: 3})
				for range s.Events {
				}
				result, err = s.FinalResult(), s.Err()
			} else {
				result, err = runner.Run(context.Background(), source, nil, RunConfig{MaxTurns: 3})
			}
			if err != nil {
				t.Fatal(err)
			}
			tools := []string{}
			for _, tool := range model.requests[0].Tools {
				tools = append(tools, tool.Name())
			}
			outputs := []map[string]any{}
			for _, item := range model.requests[1].Input {
				if item.Type == RunItemToolOutput {
					outputs = append(outputs, map[string]any{"id": item.ToolOutput.CallID, "error": item.ToolOutput.IsError, "content": item.ToolOutput.Content})
				}
			}
			want := "router"
			if after {
				want = "expert"
			}
			if name == "enabled_sibling" {
				want = "other"
			}
			if result.LastAgent.Name != want || len(model.requests) != 2 || (count == 1) != (want == "expert") {
				t.Fatalf("%s: result=%s callback=%d", name, result.LastAgent.Name, count)
			}
			cases = append(cases, map[string]any{"scenario": name, "streamed": streamed, "initial": initial, "after_request": after, "tools": tools, "last_agent": result.LastAgent.Name, "callbacks": count, "outputs": outputs, "final_text": result.FinalText()})
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
