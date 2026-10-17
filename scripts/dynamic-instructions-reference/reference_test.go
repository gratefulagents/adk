// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"testing"
)

type dynamicRetryModel struct {
	*subagentToolMockModel
	failed bool
}

func (m *dynamicRetryModel) first(req ModelRequest) bool {
	if m.failed {
		return false
	}
	m.failed = true
	m.requests = append(m.requests, req)
	return true
}
func (m *dynamicRetryModel) GetResponse(ctx context.Context, req ModelRequest) (*ModelResponse, error) {
	if m.first(req) {
		return nil, errors.New("transient")
	}
	return m.subagentToolMockModel.GetResponse(ctx, req)
}
func (m *dynamicRetryModel) StreamResponse(ctx context.Context, req ModelRequest) (*ModelStream, error) {
	if m.first(req) {
		return nil, errors.New("transient")
	}
	return m.subagentToolMockModel.StreamResponse(ctx, req)
}

func TestNativeDynamicInstructionsReference(t *testing.T) {
	cases := []map[string]any{}
	for _, name := range []string{"static", "dynamic", "blank", "compose", "progress", "handoff", "retry", "summary_retry"} {
		for _, streamed := range []bool{false, true} {
			calls := []string{}
			resolve := func(ctx *RunContext, a *Agent) string {
				text := fmt.Sprintf("dynamic:%s:%d:%d", a.Name, ctx.Usage.InputTokens, ctx.Usage.OutputTokens)
				calls = append(calls, text)
				if name == "blank" {
					return ""
				}
				return text
			}
			a := &Agent{Name: "agent", Instructions: "static fallback"}
			if name != "static" {
				a.InstructionsFn = resolve
			}
			cfg := RunConfig{MaxTurns: 4}
			if name == "compose" {
				cfg.AdditionalInstructions = " additional instructions "
				a.MCPServers = []string{"files"}
			}
			reply := func(text string) *ModelResponse {
				return &ModelResponse{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: text}}}}
			}
			responses := []*ModelResponse{reply("done")}
			if name == "progress" {
				first := reply("progress")
				no := false
				first.EndTurn = &no
				first.Usage = Usage{InputTokens: 7, OutputTokens: 3}
				responses = []*ModelResponse{first, reply("done")}
			}
			if name == "handoff" {
				target := &Agent{Name: "child", Instructions: "child fallback", InstructionsFn: resolve}
				a.Handoffs = []*Handoff{NewHandoff(target)}
				responses = []*ModelResponse{{Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "transfer", Name: "transfer_to_child", Input: json.RawMessage(`{}`)}}}, Usage: Usage{InputTokens: 7, OutputTokens: 3}}, reply("done")}
			}
			m := &subagentToolMockModel{responses: responses}
			runner := NewRunnerWithModel(m)
			if name == "retry" || name == "summary_retry" {
				runner = NewRunnerWithModel(&dynamicRetryModel{subagentToolMockModel: m})
				cfg.RetryPolicy = &RetryPolicy{MaxRetries: 1, Backoff: RetryBackoffSettings{InitialDelayMS: 1, MaxDelayMS: 1, Multiplier: 1}}
			}
			if name == "summary_retry" {
				cfg.MaxTurns = 2
				cfg.ForceFinalSummaryTurn = true
			}
			var result *RunResult
			var err error
			if streamed {
				s := runner.RunStreamed(context.Background(), a, nil, cfg)
				for range s.Events {
				}
				result = s.FinalResult()
			} else {
				result, err = runner.Run(context.Background(), a, nil, cfg)
			}
			if err != nil {
				t.Fatal(err)
			}
			instructions := []string{}
			for _, r := range m.requests {
				instructions = append(instructions, r.Instructions)
			}
			cases = append(cases, map[string]any{"name": name, "streamed": streamed, "instructions": instructions, "calls": calls, "final": result.FinalText(), "original": a.Instructions})
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
