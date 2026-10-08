// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"testing"
)

func TestNativeCriticReference(t *testing.T) {
	replies := []string{"", "checked", "VERDICT: APPROVED", " verdict: approved. \n", "VERDICT: APPROVED because done", "quoted VERDICT: APPROVED", `"VERDICT: APPROVED"`, "VERDICT: REJECTED\n1. gap", "VERDICT: REJECTEDness", "VERDICT: APPROVED\nVERDICT: REJECTED gap", "VERDICT: REJECTED gap\nVERDICT: APPROVED", "VERDICT: APPROVED\nVERDICT: inconclusive", "VERDICT: INCONCLUSIVE", "VERDICT: APPROVED..", "VERDICT : APPROVED", "\u00a0verdict:\u00a0approved.\u00a0", "verdıct: approved", "```\nVERDICT: APPROVED\n```", "VERDICT: Rejected\n1. gap \n", "VERDICT: APPROVED\nVERDICT: APPROVED.", "  VERDICT:\tAPPROVED  "}
	type specification struct {
		name, reply, instructions string
		structured                bool
	}
	var specs []specification
	for i, reply := range replies {
		specs = append(specs, specification{fmt.Sprintf("verdict_%02d", i), reply, "", false})
	}
	for _, name := range []string{"custom", "blank", "read", "write", "turn_cap", "error", "structured", "structured_string"} {
		item := specification{name: name, reply: "VERDICT: APPROVED"}
		if name == "custom" {
			item.instructions = " custom instructions \n"
		}
		if name == "blank" {
			item.instructions = " \n\t"
		}
		if name == "structured" {
			item.structured = true
			item.reply = `{"verdict":"VERDICT: APPROVED"}`
		}
		if name == "structured_string" {
			item.structured = true
			item.reply = `"VERDICT: APPROVED"`
		}
		specs = append(specs, item)
	}
	var cases []map[string]any
	for _, spec := range specs {
		readCalls, writeCalls := 0, 0
		nestedTurns := []int{}
		read := &FunctionTool{ToolName: "read", Schema: json.RawMessage(`{}`), ReadOnly: true, Fn: func(ctx context.Context, _ json.RawMessage) (string, error) {
			readCalls++
			cfg, ok := NestedRunConfigFromContext(ctx)
			if !ok {
				t.Fatal("missing nested configuration")
			}
			nestedTurns = append(nestedTurns, cfg.MaxTurns)
			return "ok", nil
		}}
		write := &FunctionTool{ToolName: "write", Schema: json.RawMessage(`{}`), Fn: func(context.Context, json.RawMessage) (string, error) { writeCalls++; return "unsafe", nil }}
		a := &Agent{Name: "critic", Instructions: spec.instructions, Tools: []Tool{read, write}}
		if spec.structured {
			a.OutputType = &OutputSchema{Schema: json.RawMessage(`true`)}
		}
		model := &subagentToolMockModel{}
		if spec.name == "read" || spec.name == "write" || spec.name == "turn_cap" {
			count := 1
			if spec.name == "turn_cap" {
				count = 11
			}
			name := spec.name
			if name == "turn_cap" {
				name = "read"
			}
			for i := 0; i < count; i++ {
				model.responses = append(model.responses, &ModelResponse{Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: fmt.Sprintf("call_%d", i), Name: name, Input: json.RawMessage(`{}`)}}}})
			}
		}
		if spec.name != "error" {
			model.responses = append(model.responses, &ModelResponse{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: spec.reply}}}})
		}
		task := "original <task>\n第二行"
		candidate := "candidate <&>\nnext"
		verify := NewCriticVerifier(NewRunnerWithModel(model), a, task)
		feedback, err := verify(context.Background(), candidate)
		instructions := []string{}
		tools := [][]string{}
		prompt := ""
		for _, req := range model.requests {
			instructions = append(instructions, req.Instructions)
			names := []string{}
			for _, tool := range req.Tools {
				names = append(names, tool.Name())
			}
			tools = append(tools, names)
			if prompt == "" && len(req.Input) > 0 {
				prompt = req.Input[0].Message.Text
			}
		}
		cases = append(cases, map[string]any{"name": spec.name, "reply": spec.reply, "instructions": spec.instructions, "structured": spec.structured, "task": task, "candidate": candidate, "feedback": feedback, "error": err != nil, "requests": len(model.requests), "request_instructions": instructions, "request_tools": tools, "prompt": prompt, "original_instructions": a.Instructions, "read_calls": readCalls, "write_calls": writeCalls, "nested_turns": nestedTurns})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases, "default_instructions": DefaultCriticInstructions}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
