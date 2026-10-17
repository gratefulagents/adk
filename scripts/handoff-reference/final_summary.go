package main

import (
	"context"
	"encoding/json"
	"fmt"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type summaryRequest struct {
	Instructions string   `json:"instructions"`
	Tools        []string `json:"tools"`
}
type summaryModel struct {
	offlineModel
	turns    int
	requests []summaryRequest
}

func (m *summaryModel) GetResponse(_ context.Context, request sdk.ModelRequest) (*sdk.ModelResponse, error) {
	observation := summaryRequest{Instructions: request.Instructions, Tools: []string{}}
	for _, tool := range request.Tools {
		observation.Tools = append(observation.Tools, tool.Name())
	}
	m.requests = append(m.requests, observation)
	if len(m.requests) > m.turns {
		return nil, fmt.Errorf("unexpected model call")
	}
	if len(m.requests) < m.turns {
		return &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "read", Name: "inspect", Input: json.RawMessage(`{}`)}}}}, nil
	}
	return &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "summary"}}}}, nil
}
func (m *summaryModel) StreamResponse(ctx context.Context, request sdk.ModelRequest) (*sdk.ModelStream, error) {
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
func (*summaryModel) CalculateCost(sdk.Usage) float64 { return 0 }

type summaryTool struct {
	observationTool
	calls int
}

func (t *summaryTool) Execute(context.Context, json.RawMessage, string) (sdk.ToolResult, error) {
	t.calls++
	return sdk.ToolResult{Content: "raw inspect"}, nil
}

type summaryCase struct {
	Enabled   bool             `json:"enabled"`
	Turns     int              `json:"turns"`
	Streaming bool             `json:"streaming"`
	Requests  []summaryRequest `json:"requests"`
	ToolCalls int              `json:"tool_calls"`
	Output    any              `json:"output"`
}

func generateFinalSummary() ([]byte, error) {
	fixture := struct {
		SchemaVersion int           `json:"schema_version"`
		SDKRevision   string        `json:"sdk_revision"`
		Cases         []summaryCase `json:"cases"`
	}{SchemaVersion: 1, SDKRevision: sdkRevision}
	for _, enabled := range []bool{false, true} {
		for _, turns := range []int{1, 2} {
			for _, streaming := range []bool{false, true} {
				model := &summaryModel{turns: turns, requests: []summaryRequest{}}
				tool := &summaryTool{observationTool: observationTool{name: "inspect", readOnly: true}}
				agent := &sdk.Agent{Name: "test", Model: "offline", Instructions: "stable instructions", Tools: []sdk.Tool{tool}}
				runner := sdk.NewRunnerWithModel(model)
				cfg := sdk.RunConfig{MaxTurns: turns, ForceFinalSummaryTurn: enabled, TracingDisabled: true, ModelCallTimeout: -1}
				var result *sdk.RunResult
				var err error
				if streaming {
					stream := runner.RunStreamed(context.Background(), agent, nil, cfg)
					for range stream.Events {
					}
					result, err = stream.FinalResult(), stream.Err()
				} else {
					result, err = runner.Run(context.Background(), agent, nil, cfg)
				}
				if err != nil {
					return nil, err
				}
				fixture.Cases = append(fixture.Cases, summaryCase{enabled, turns, streaming, model.requests, tool.calls, result.FinalOutput})
			}
		}
	}
	b, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
