// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func TestNativeConfirmationReference(t *testing.T) {
	var cases []map[string]any
	for _, name := range []string{"double", "tools", "end_turn", "off", "no_tools", "summary", "gate", "poll", "finalize"} {
		for _, streamed := range []bool{false, true} {
			reply := func(text string) *ModelResponse {
				return &ModelResponse{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: text}}}}
			}
			replies := []*ModelResponse{reply("first"), reply("final")}
			cfg := RunConfig{MaxTurns: 1, RequireCompletionConfirmation: true}
			switch name {
			case "off":
				cfg.RequireCompletionConfirmation = false
			case "summary":
				cfg.ForceFinalSummaryTurn = true
			case "tools":
				cfg.MaxTurns = 3
				replies = []*ModelResponse{reply("first"), {Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "read", Name: "read", Input: json.RawMessage(`{}`)}}}}, reply("again"), reply("final")}
			case "end_turn":
				cfg.MaxTurns = 3
				progress := reply("progress")
				no := false
				progress.EndTurn = &no
				replies = []*ModelResponse{reply("first"), progress, reply("again"), reply("final")}
			case "gate", "poll":
				replies = []*ModelResponse{reply("first"), reply("again"), reply("final")}
			case "finalize":
				replies = []*ModelResponse{reply("first"), reply("again"), reply("after input"), reply("final")}
			}
			gateCalls := 0
			if name == "gate" {
				cfg.StopGateMaxBlocks = 1
				cfg.StopGate = func(context.Context, string) (bool, string) { gateCalls++; return false, "check" }
			}
			pollCalls := 0
			if name == "poll" {
				cfg.ImmediateInputPoller = func(context.Context) ([]RunItem, error) {
					pollCalls++
					if pollCalls == 2 {
						return []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "steering"}}}, nil
					}
					return nil, nil
				}
			}
			finalizerCalls := 0
			if name == "finalize" {
				cfg.ImmediateInputFinalizer = func(context.Context) ([]RunItem, error) {
					finalizerCalls++
					if finalizerCalls == 1 {
						return []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "steering"}}}, nil
					}
					return nil, nil
				}
			}
			m := &subagentToolMockModel{responses: replies}
			a := &Agent{Name: "agent"}
			if name != "no_tools" {
				a.Tools = []Tool{&FunctionTool{ToolName: "read", Schema: json.RawMessage(`{}`), Fn: func(context.Context, json.RawMessage) (string, error) { return "ok", nil }}}
			}
			runner := NewRunnerWithModel(m)
			var result *RunResult
			if streamed {
				stream := runner.RunStreamed(context.Background(), a, nil, cfg)
				for range stream.Events {
				}
				result = stream.FinalResult()
			} else {
				var err error
				result, err = runner.Run(context.Background(), a, nil, cfg)
				if err != nil {
					t.Fatalf("%s: %v", name, err)
				}
			}
			if result == nil {
				t.Fatalf("%s: missing result", name)
			}
			feedback := []string{}
			for _, item := range result.NewItems {
				if item.Message != nil && strings.HasPrefix(item.Message.Text, "[SYSTEM]") {
					feedback = append(feedback, item.Message.Text)
				}
			}
			tools := []int{}
			for _, req := range m.requests {
				tools = append(tools, len(req.Tools))
			}
			cases = append(cases, map[string]any{"name": name, "streamed": streamed, "calls": len(m.requests), "tools": tools, "final": result.FinalText(), "feedback": feedback, "gate_calls": gateCalls, "poll_calls": pollCalls, "finalizer_calls": finalizerCalls})
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
