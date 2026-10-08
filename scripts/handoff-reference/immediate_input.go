package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type immediateItem struct {
	Type       string              `json:"type"`
	Agent      *string             `json:"agent"`
	Message    string              `json:"message,omitempty"`
	ToolCall   *sdk.ToolCallData   `json:"tool_call,omitempty"`
	ToolOutput *sdk.ToolOutputData `json:"tool_output,omitempty"`
}

func projectImmediateItems(items []sdk.RunItem) []immediateItem {
	out := make([]immediateItem, 0, len(items))
	for _, item := range items {
		projected := immediateItem{}
		if item.Agent != nil {
			name := item.Agent.Name
			projected.Agent = &name
		}
		switch item.Type {
		case sdk.RunItemMessage:
			projected.Type, projected.Message = "message", item.Message.Text
		case sdk.RunItemToolCall:
			projected.Type, projected.ToolCall = "tool_call", item.ToolCall
		case sdk.RunItemToolOutput:
			projected.Type, projected.ToolOutput = "tool_output", item.ToolOutput
		default:
			panic(fmt.Sprintf("unexpected immediate-input item type %d", item.Type))
		}
		out = append(out, projected)
	}
	return out
}

type immediateRequest struct {
	Input []immediateItem `json:"input"`
}

type immediateModel struct {
	offlineModel
	responses [][]sdk.RunItem
	requests  []immediateRequest
}

func (m *immediateModel) GetResponse(_ context.Context, request sdk.ModelRequest) (*sdk.ModelResponse, error) {
	m.requests = append(m.requests, immediateRequest{Input: projectImmediateItems(request.Input)})
	if len(m.requests) > len(m.responses) {
		return nil, fmt.Errorf("unexpected immediate-input model call %d", len(m.requests))
	}
	return &sdk.ModelResponse{Items: m.responses[len(m.requests)-1]}, nil
}

func (m *immediateModel) StreamResponse(ctx context.Context, request sdk.ModelRequest) (*sdk.ModelStream, error) {
	response, err := m.GetResponse(ctx, request)
	if err != nil {
		return nil, err
	}
	events := make(chan sdk.ModelStreamEvent, 1)
	done := make(chan *sdk.ModelResponse, 1)
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: response}
	done <- response
	close(events)
	close(done)
	return sdk.NewModelStream(events, done), nil
}

func (*immediateModel) CalculateCost(sdk.Usage) float64 { return 0 }

type immediateTool struct {
	observationTool
	calls int
}

func (t *immediateTool) Execute(context.Context, json.RawMessage, string) (sdk.ToolResult, error) {
	t.calls++
	return sdk.ToolResult{Content: "tool result"}, nil
}

type immediateCase struct {
	Name           string             `json:"name"`
	Streaming      bool               `json:"streaming"`
	MaxTurns       int                `json:"max_turns"`
	Requests       []immediateRequest `json:"requests"`
	PollCalls      int                `json:"poll_calls"`
	FinalizerCalls int                `json:"finalizer_calls"`
	ToolCalls      int                `json:"tool_calls"`
	Output         any                `json:"output"`
	Error          string             `json:"error"`
	NewItems       []immediateItem    `json:"new_items"`
	History        []immediateItem    `json:"history"`
}

type immediateFixture struct {
	SchemaVersion int             `json:"schema_version"`
	SDKRevision   string          `json:"sdk_revision"`
	Cases         []immediateCase `json:"cases"`
}

func immediateMessage(text string) sdk.RunItem {
	return sdk.RunItem{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: text}}
}

func generateImmediateInput() ([]byte, error) {
	fixture := immediateFixture{SchemaVersion: 1, SDKRevision: sdkRevision}
	for _, scenario := range []struct {
		name           string
		maxTurns       int
		pollAt         int
		pollError      bool
		lateInput      bool
		finalizerError bool
		firstTool      bool
	}{
		{name: "poll_before_first_request", maxTurns: 3, pollAt: 1},
		{name: "poll_after_tool_response", maxTurns: 3, pollAt: 2, firstTool: true},
		{name: "poll_error_best_effort", maxTurns: 3, pollAt: 1, pollError: true},
		{name: "finalizer_late_input", maxTurns: 3, lateInput: true},
		{name: "empty_finalizer_ends", maxTurns: 3},
		{name: "finalizer_error_propagated", maxTurns: 3, finalizerError: true},
		{name: "max_turns_finalizer_extends", maxTurns: 1, lateInput: true, firstTool: true},
	} {
		for _, streaming := range []bool{false, true} {
			observation := immediateCase{Name: scenario.name, Streaming: streaming, MaxTurns: scenario.maxTurns, Error: "none"}
			model := &immediateModel{responses: [][]sdk.RunItem{{immediateMessage("answer-1")}}}
			if scenario.firstTool {
				model.responses[0] = []sdk.RunItem{{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "call-1", Name: "inspect", Input: json.RawMessage(`{}`)}}}
			}
			if scenario.firstTool || scenario.lateInput {
				model.responses = append(model.responses, []sdk.RunItem{immediateMessage("answer-2")})
			}
			tool := &immediateTool{observationTool: observationTool{name: "inspect", readOnly: true}}
			agent := &sdk.Agent{Name: "oracle", Model: "offline", Tools: []sdk.Tool{tool}}
			finalizerError := errors.New("oracle finalizer failure")
			cfg := sdk.RunConfig{
				MaxTurns: scenario.maxTurns, TracingDisabled: true, ModelCallTimeout: -1,
				ImmediateInputPoller: func(context.Context) ([]sdk.RunItem, error) {
					observation.PollCalls++
					if observation.PollCalls != scenario.pollAt {
						return nil, nil
					}
					if scenario.pollError {
						return []sdk.RunItem{immediateMessage("discarded")}, errors.New("oracle poll failure")
					}
					return []sdk.RunItem{immediateMessage("polled-1"), immediateMessage("polled-2")}, nil
				},
				ImmediateInputFinalizer: func(context.Context) ([]sdk.RunItem, error) {
					observation.FinalizerCalls++
					if scenario.finalizerError {
						return nil, finalizerError
					}
					if scenario.lateInput && observation.FinalizerCalls == 1 {
						return []sdk.RunItem{immediateMessage("late")}, nil
					}
					return nil, nil
				},
			}
			runner := sdk.NewRunnerWithModel(model)
			input := []sdk.RunItem{immediateMessage("initial")}
			var result *sdk.RunResult
			var err error
			if streaming {
				stream := runner.RunStreamed(context.Background(), agent, input, cfg)
				for range stream.Events {
				}
				result, err = stream.FinalResult(), stream.Err()
			} else {
				result, err = runner.Run(context.Background(), agent, input, cfg)
			}
			if errors.Is(err, finalizerError) {
				observation.Error = "finalizer_error"
			} else if err != nil {
				return nil, fmt.Errorf("%s streaming=%t: %w", scenario.name, streaming, err)
			}
			if result == nil {
				return nil, fmt.Errorf("%s streaming=%t: missing result", scenario.name, streaming)
			}
			observation.Requests, observation.ToolCalls = model.requests, tool.calls
			observation.Output = result.FinalOutput
			observation.NewItems = projectImmediateItems(result.NewItems)
			observation.History = projectImmediateItems(result.FinalHistory)
			fixture.Cases = append(fixture.Cases, observation)
		}
	}
	b, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
