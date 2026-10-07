// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"testing"
)

func TestNativeVerifierReference(t *testing.T) {
	var cases []map[string]any
	for _, name := range []string{"approve", "blank", "reject", "error", "no_tools", "summary", "confirmation", "gate", "tools", "end_turn", "object", "number", "null", "string"} {
		for _, streamed := range []bool{false, true} {
			reply := func(text string) *ModelResponse {
				return &ModelResponse{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: text}}}}
			}
			replies := []*ModelResponse{reply("first"), reply("final")}
			cfg := RunConfig{MaxTurns: 1}
			feedback := "fix this"
			switch name {
			case "approve", "object", "number", "null", "string":
				feedback = ""
			case "blank":
				feedback = " \n"
			case "summary":
				cfg.ForceFinalSummaryTurn = true
			case "confirmation":
				cfg.RequireCompletionConfirmation = true
				replies = []*ModelResponse{reply("first"), reply("confirmed"), reply("revised"), reply("final")}
			case "gate":
				replies = []*ModelResponse{reply("first"), reply("second"), reply("final")}
			case "tools":
				cfg.MaxTurns = 3
				replies = []*ModelResponse{reply("first"), {Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "read", Name: "read", Input: json.RawMessage(`{}`)}}}}, reply("final")}
			case "end_turn":
				cfg.MaxTurns = 3
				progress := reply("progress")
				no := false
				progress.EndTurn = &no
				replies = []*ModelResponse{reply("first"), progress, reply("final")}
			}
			seen := []string{}
			cfg.FinalAnswerVerifier = func(_ context.Context, text string) (string, error) {
				seen = append(seen, text)
				if name == "error" {
					return "ignored feedback", errors.New("private verifier failure")
				}
				return feedback, nil
			}
			gateCalls := 0
			if name == "gate" {
				cfg.StopGateMaxBlocks = 1
				cfg.StopGate = func(context.Context, string) (bool, string) { gateCalls++; return false, "check" }
			}
			a := &Agent{Name: "agent"}
			if name != "no_tools" {
				a.Tools = []Tool{&FunctionTool{ToolName: "read", Schema: json.RawMessage(`{}`), Fn: func(context.Context, json.RawMessage) (string, error) { return "ok", nil }}}
			}
			if raw, ok := map[string]string{"object": `{"z":"<&>","a":1}`, "number": `42`, "null": `null`, "string": `"decoded"`}[name]; ok {
				a.OutputType = &OutputSchema{Schema: json.RawMessage(`true`)}
				replies = []*ModelResponse{reply(raw)}
			}
			model := &subagentToolMockModel{responses: replies}
			runner := NewRunnerWithModel(model)
			var result *RunResult
			if streamed {
				s := runner.RunStreamed(context.Background(), a, nil, cfg)
				for range s.Events {
				}
				result = s.FinalResult()
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
			messages := []string{}
			for _, item := range result.NewItems {
				if item.Message != nil && strings.HasPrefix(item.Message.Text, "[SYSTEM]") {
					messages = append(messages, item.Message.Text)
				}
			}
			tools := []int{}
			for _, req := range model.requests {
				tools = append(tools, len(req.Tools))
			}
			cases = append(cases, map[string]any{"name": name, "streamed": streamed, "calls": len(model.requests), "verifier_inputs": seen, "gate_calls": gateCalls, "feedback": messages, "final_output": result.FinalOutput, "tools": tools})
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
