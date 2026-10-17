package main

import (
	"context"
	"encoding/json"
	"strings"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type cacheModel struct {
	offlineModel
	keys []string
}

func (m *cacheModel) GetResponse(_ context.Context, request sdk.ModelRequest) (*sdk.ModelResponse, error) {
	m.keys = append(m.keys, request.PromptCacheKey)
	return &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "done"}}}}, nil
}

func (m *cacheModel) StreamResponse(ctx context.Context, request sdk.ModelRequest) (*sdk.ModelStream, error) {
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

func (*cacheModel) CalculateCost(sdk.Usage) float64 { return 0 }

type cacheCase struct {
	TraceID   string   `json:"trace_id"`
	Namespace string   `json:"namespace"`
	Logical   string   `json:"logical"`
	Streaming bool     `json:"streaming"`
	Keys      []string `json:"keys"`
}

func generatePromptCache() ([]byte, error) {
	fixture := struct {
		SchemaVersion int         `json:"schema_version"`
		SDKRevision   string      `json:"sdk_revision"`
		Cases         []cacheCase `json:"cases"`
	}{SchemaVersion: 1, SDKRevision: sdkRevision}
	for _, traceID := range []string{"run-1", "other-root"} {
		for _, namespace := range []string{"", " \t\u0085", "\u2003namespace-a\u3000", "namespace-b", "ns\x00tail", "../../evil/💥"} {
			for _, logical := range []string{"", "\t \u0085", "\u2003logical\u3000", "task-stable-123", "key\x00tail", "../../evil/💥", "[SYSTEM]", strings.Repeat("../../evil/💥", 100), "child"} {
				for _, streaming := range []bool{false, true} {
					model := &cacheModel{keys: []string{}}
					runner := sdk.NewRunnerWithModel(model)
					agent := &sdk.Agent{Name: "parent", Model: "offline"}
					input := []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "go"}}}
					cfg := sdk.RunConfig{MaxTurns: 2, Trace: &sdk.Trace{ID: traceID}, PromptCacheNamespace: namespace, PromptCacheKey: logical, TracingDisabled: true, ModelCallTimeout: -1}
					for range 2 {
						var err error
						if streaming {
							stream := runner.RunStreamed(context.Background(), agent, input, cfg)
							for range stream.Events {
							}
							err = stream.Err()
						} else {
							_, err = runner.Run(context.Background(), agent, input, cfg)
						}
						if err != nil {
							return nil, err
						}
					}
					fixture.Cases = append(fixture.Cases, cacheCase{traceID, namespace, logical, streaming, model.keys})
				}
			}
		}
	}
	encoded, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(encoded, '\n'), nil
}
