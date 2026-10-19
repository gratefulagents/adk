// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func filterProjection(items []RunItem) []map[string]any {
	out := []map[string]any{}
	for _, item := range items {
		switch item.Type {
		case RunItemMessage:
			out = append(out, map[string]any{"kind": "message", "text": item.Message.Text})
		case RunItemToolCall:
			out = append(out, map[string]any{"kind": "call", "id": item.ToolCall.ID, "name": item.ToolCall.Name})
		case RunItemToolOutput:
			out = append(out, map[string]any{"kind": "output", "id": item.ToolOutput.CallID, "text": item.ToolOutput.Content, "error": item.ToolOutput.IsError})
		}
	}
	return out
}
func TestNativeHandoffFilterReference(t *testing.T) {
	cases := []map[string]any{}
	for _, streamed := range []bool{false, true} {
		for _, name := range []string{"preserve", "messages", "replace", "empty"} {
			target := &Agent{Name: "expert"}
			handoff := NewHandoff(target)
			events := []string{}
			handoff.OnHandoff = func(*RunContext, json.RawMessage) { events = append(events, "callback") }
			var inputSeen, allSeen []map[string]any
			handoff.InputFilter = func(input, all []RunItem) []RunItem {
				events = append(events, "filter")
				inputSeen, allSeen = filterProjection(input), filterProjection(all)
				switch name {
				case "messages":
					result := []RunItem{}
					for _, item := range input {
						if item.Type == RunItemMessage {
							result = append(result, item)
						}
					}
					return result
				case "replace":
					return []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "filtered summary"}}}
				case "empty":
					return nil
				default:
					return input
				}
			}
			source := &Agent{Name: "router", Handoffs: []*Handoff{handoff}}
			calls := []RunItem{
				{Type: RunItemMessage, Message: &MessageOutput{Text: "transfer"}},
				{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "lookup", Name: "lookup", Input: json.RawMessage(`{}`)}},
				{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: handoff.ToolName, Input: json.RawMessage(`{}`)}},
			}
			model := &subagentToolMockModel{responses: []*ModelResponse{{Items: calls}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}
			runner := NewRunnerWithModel(model)
			input := []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "hello"}}}
			var result *RunResult
			var err error
			if streamed {
				s := runner.RunStreamed(context.Background(), source, input, RunConfig{MaxTurns: 3})
				for range s.Events {
				}
				result, err = s.FinalResult(), s.Err()
			} else {
				result, err = runner.Run(context.Background(), source, input, RunConfig{MaxTurns: 3})
			}
			if err != nil {
				t.Fatal(err)
			}
			if result.LastAgent != target || len(model.requests) != 2 || len(inputSeen) != 6 || len(allSeen) != 5 {
				t.Fatalf("%s: input=%v all=%v", name, inputSeen, allSeen)
			}
			cases = append(cases, map[string]any{"scenario": name, "streamed": streamed, "events": events, "filter_input": inputSeen, "filter_all": allSeen, "target_input": filterProjection(model.requests[1].Input), "result_items": filterProjection(result.NewItems), "final_history": filterProjection(result.FinalHistory), "last_agent": result.LastAgent.Name})
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
