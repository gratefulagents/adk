// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../crates/adk-runtime/tests/fixtures/boundaries.go
package main

import (
	"context"
	"encoding/json"
	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
	"strings"
	"time"
)

type model struct {
	responses []*sdk.ModelResponse
	calls     int
}

func (m *model) GetResponse(context.Context, sdk.ModelRequest) (*sdk.ModelResponse, error) {
	m.calls++
	r := m.responses[0]
	m.responses = m.responses[1:]
	return r, nil
}
func (m *model) StreamResponse(ctx context.Context, req sdk.ModelRequest) (*sdk.ModelStream, error) {
	r, _ := m.GetResponse(ctx, req)
	events := make(chan sdk.ModelStreamEvent, 1)
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: r}
	close(events)
	done := make(chan *sdk.ModelResponse, 1)
	done <- r
	close(done)
	return sdk.NewModelStream(events, done), nil
}
func (*model) GetRetryAdvice(error) *sdk.ModelRetryAdvice { return nil }
func (*model) CalculateCost(sdk.Usage) float64            { return 0 }
func (*model) Provider() string                           { return "fixture" }
func main() {
	if len(os.Args) > 1 && os.Args[1] == "stop-gate" {
		stopGateFixture()
		return
	}
	output := map[string][]sdk.DurableCheckpoint{}
	for _, mode := range []string{"tool", "pause", "handoff", "approval"} {
		name := "effect"
		if mode == "pause" {
			name = "AskUserQuestion"
		}
		agent := &sdk.Agent{Name: "agent"}
		if mode == "handoff" {
			h := sdk.NewHandoff(&sdk.Agent{Name: "target"})
			name = h.ToolName
			agent.Handoffs = []*sdk.Handoff{h}
		} else {
			agent.Tools = []sdk.Tool{&sdk.FunctionTool{ToolName: name, ReadOnly: true, Approval: mode == "approval", Fn: func(context.Context, json.RawMessage) (string, error) { return "done", nil }}}
		}
		m := &model{responses: []*sdk.ModelResponse{{Items: []sdk.RunItem{{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "call-1", Name: name, Input: json.RawMessage(`{}`)}}}}, {Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "done"}}}}}}
		_, err := sdk.NewRunnerWithModel(m).Run(context.Background(), agent, nil, sdk.RunConfig{MaxTurns: 4, Durable: &sdk.DurableRunConfig{RunID: "run-1", AttemptID: "go-attempt", Checkpoint: func(_ context.Context, cp sdk.DurableCheckpoint) error {
			cp.StepID = "step_go"
			cp.CreatedAt = time.Date(2026, 1, 2, 3, 4, 5, 0, time.UTC)
			output[mode] = append(output[mode], cp)
			return nil
		}}})
		if err != nil {
			panic(err)
		}
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if err := e.Encode(output); err != nil {
		panic(err)
	}
}

func stopGateFixture() {
	output := map[string]any{}
	for _, reset := range []bool{false, true} {
		candidate := &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "candidate"}}}}
		m := &model{responses: []*sdk.ModelResponse{candidate, candidate, candidate}}
		maxTurns := 1
		name := "cap"
		if reset {
			name = "tool_reset"
			maxTurns = 8
			m.responses = []*sdk.ModelResponse{candidate, {Items: []sdk.RunItem{{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "one", Name: "effect", Input: json.RawMessage(`{}`)}}}}, candidate, candidate, candidate}
		}
		agent := &sdk.Agent{Name: "agent", Tools: []sdk.Tool{&sdk.FunctionTool{ToolName: "effect", ReadOnly: true, Fn: func(context.Context, json.RawMessage) (string, error) { return "done", nil }}}}
		gateCalls := 0
		result, err := sdk.NewRunnerWithModel(m).Run(context.Background(), agent, nil, sdk.RunConfig{
			MaxTurns: maxTurns, StopGateMaxBlocks: 2,
			StopGate: func(context.Context, string) (bool, string) { gateCalls++; return false, "" },
			Durable:  &sdk.DurableRunConfig{RunID: "run-1", AttemptID: "go-attempt", Checkpoint: func(context.Context, sdk.DurableCheckpoint) error { return nil }},
		})
		if err != nil {
			panic(err)
		}
		feedback := []string{}
		for _, item := range result.FinalHistory {
			if item.Message != nil && strings.HasPrefix(item.Message.Text, "[SYSTEM] Final answer blocked") {
				feedback = append(feedback, item.Message.Text)
			}
		}
		output[name] = map[string]any{"calls": m.calls, "gate_calls": gateCalls, "max_turns": maxTurns, "final_output": result.FinalText(), "feedback": feedback}
	}
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if err := e.Encode(output); err != nil {
		panic(err)
	}
}
