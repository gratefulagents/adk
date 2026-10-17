package main

import (
	"context"
	"encoding/json"
	"fmt"
	"sync"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type immediateSignalCallback struct {
	Name  string          `json:"name"`
	Items []immediateItem `json:"items"`
}

type immediateSignalDelta struct {
	Name string `json:"name"`
	Text string `json:"text"`
}

type immediateSignalCase struct {
	Name                 string                    `json:"name"`
	Streaming            bool                      `json:"streaming"`
	MaxTurns             int                       `json:"max_turns"`
	SignalAfter          string                    `json:"signal_after"`
	RequestCount         int                       `json:"request_count"`
	Requests             []immediateRequest        `json:"requests"`
	FirstAttemptCanceled bool                      `json:"first_attempt_canceled"`
	RetryAdviceCalls     int                       `json:"retry_advice_calls"`
	PollCalls            int                       `json:"poll_calls"`
	FinalizerCalls       int                       `json:"finalizer_calls"`
	Callbacks            []immediateSignalCallback `json:"callbacks"`
	ToolCalls            int                       `json:"tool_calls"`
	Output               any                       `json:"output"`
	Error                string                    `json:"error"`
	AcceptedResponses    [][]immediateItem         `json:"accepted_responses"`
	NewItems             []immediateItem           `json:"new_items"`
	History              []immediateItem           `json:"history"`
	ConsumedStreamItems  []immediateItem           `json:"consumed_stream_items"`
	ConsumedStreamDeltas []immediateSignalDelta    `json:"consumed_stream_deltas"`
}

type immediateSignalFixture struct {
	SchemaVersion int                   `json:"schema_version"`
	SDKRevision   string                `json:"sdk_revision"`
	Cases         []immediateSignalCase `json:"cases"`
}

type immediateSignalModel struct {
	offlineModel
	requests             []immediateRequest
	ready                chan struct{}
	release              chan struct{}
	workers              sync.WaitGroup
	visible              string
	firstTool            bool
	firstAttemptCanceled bool
	retryAdviceCalls     int
}

func (m *immediateSignalModel) GetRetryAdvice(error) *sdk.ModelRetryAdvice {
	m.retryAdviceCalls++
	return nil
}

func (*immediateSignalModel) CalculateCost(sdk.Usage) float64 { return 0 }

func (m *immediateSignalModel) StreamResponse(ctx context.Context, request sdk.ModelRequest) (*sdk.ModelStream, error) {
	m.requests = append(m.requests, immediateRequest{Input: projectImmediateItems(request.Input)})
	attempt := len(m.requests)
	if attempt > 2 {
		return nil, fmt.Errorf("unexpected signal model request %d", attempt)
	}
	response := &sdk.ModelResponse{Items: []sdk.RunItem{immediateMessage("answer-2")}}
	events := make(chan sdk.ModelStreamEvent, 1)
	done := make(chan *sdk.ModelResponse, 1)
	if attempt == 1 && m.visible == "" {
		close(m.ready)
		<-ctx.Done()
		m.firstAttemptCanceled = true
		// A provider may finish despite cancellation; the runner must discard it.
		response.Items = []sdk.RunItem{immediateMessage("hidden-superseded-answer")}
	} else if attempt == 1 {
		response.Items = []sdk.RunItem{immediateMessage("answer-1")}
		if m.firstTool {
			response.Items = []sdk.RunItem{{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "call-1", Name: "inspect", Input: json.RawMessage(`{}`)}}}
		}
		m.workers.Add(1)
		go func() {
			defer m.workers.Done()
			defer close(events)
			defer close(done)
			kind := sdk.ModelStreamDelta
			if m.visible == "model.reasoning_delta" {
				kind = sdk.ModelStreamReasoningDelta
			}
			select {
			case events <- sdk.ModelStreamEvent{Type: kind, Delta: "visible-first-attempt"}:
			case <-ctx.Done():
				m.firstAttemptCanceled = true
				return
			}
			select {
			case <-m.release:
			case <-ctx.Done():
				m.firstAttemptCanceled = true
				return
			}
			m.firstAttemptCanceled = ctx.Err() != nil
			done <- response
			select {
			case events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: response}:
			case <-ctx.Done():
			}
		}()
		return sdk.NewModelStream(events, done), nil
	}
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: response}
	done <- response
	close(events)
	close(done)
	return sdk.NewModelStream(events, done), nil
}

func runImmediateSignalCase(name string, streaming bool, maxTurns int, visible string, firstTool bool) (immediateSignalCase, error) {
	observation := immediateSignalCase{
		Name: name, Streaming: streaming, MaxTurns: maxTurns, SignalAfter: "pending_model_attempt", Error: "none",
		ConsumedStreamItems: []immediateItem{}, ConsumedStreamDeltas: []immediateSignalDelta{},
	}
	if visible != "" {
		observation.SignalAfter = "host_received_" + visible
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	model := &immediateSignalModel{ready: make(chan struct{}), release: make(chan struct{}), visible: visible, firstTool: firstTool}
	signal := make(chan struct{})
	var queueMu sync.Mutex
	var queue []sdk.RunItem
	drain := func(name string) []sdk.RunItem {
		queueMu.Lock()
		defer queueMu.Unlock()
		items := queue
		queue = nil
		observation.Callbacks = append(observation.Callbacks, immediateSignalCallback{Name: name, Items: projectImmediateItems(items)})
		return items
	}
	hostDone := make(chan struct{})
	go func() {
		defer close(hostDone)
		select {
		case <-model.ready:
		case <-ctx.Done():
			return
		}
		queueMu.Lock()
		queue = []sdk.RunItem{immediateMessage("steer-1"), immediateMessage("steer-2")}
		queueMu.Unlock()
		// Unbuffered delivery acknowledges SDK receipt before releasing the model.
		select {
		case signal <- struct{}{}:
			close(model.release)
		case <-ctx.Done():
		}
	}()
	tool := &immediateTool{observationTool: observationTool{name: "inspect", readOnly: true}}
	agent := &sdk.Agent{Name: "oracle", Model: "offline", Tools: []sdk.Tool{tool}}
	cfg := sdk.RunConfig{
		MaxTurns: maxTurns, TracingDisabled: true, ModelCallTimeout: -1,
		ImmediateInputSignal: signal,
		ImmediateInputPoller: func(context.Context) ([]sdk.RunItem, error) {
			observation.PollCalls++
			return drain("poll"), nil
		},
		ImmediateInputFinalizer: func(context.Context) ([]sdk.RunItem, error) {
			observation.FinalizerCalls++
			return drain("finalizer"), nil
		},
	}
	runner := sdk.NewRunnerWithModel(model)
	var result *sdk.RunResult
	var err error
	input := []sdk.RunItem{immediateMessage("initial")}
	if streaming {
		stream := runner.RunStreamed(ctx, agent, input, cfg)
		visibleSeen := false
		for event := range stream.Events {
			if event.Name == "model.delta" || event.Name == "model.reasoning_delta" {
				observation.ConsumedStreamDeltas = append(observation.ConsumedStreamDeltas, immediateSignalDelta{Name: event.Name, Text: event.Delta})
				if event.Name == visible && !visibleSeen {
					visibleSeen = true
					// Sending the provider delta is insufficient: the host must see the SDK event.
					close(model.ready)
				}
			}
			if event.Type == sdk.StreamEventRunItem && event.Item != nil {
				observation.ConsumedStreamItems = append(observation.ConsumedStreamItems, projectImmediateItems([]sdk.RunItem{*event.Item})...)
			}
		}
		result, err = stream.FinalResult(), stream.Err()
	} else {
		result, err = runner.Run(ctx, agent, input, cfg)
	}
	cancel()
	<-hostDone
	model.workers.Wait()
	if err != nil {
		return observation, fmt.Errorf("%s streaming=%t: %w", name, streaming, err)
	}
	if result == nil {
		return observation, fmt.Errorf("%s: missing result", name)
	}
	observation.Requests, observation.RequestCount = model.requests, len(model.requests)
	observation.FirstAttemptCanceled, observation.RetryAdviceCalls = model.firstAttemptCanceled, model.retryAdviceCalls
	observation.ToolCalls, observation.Output = tool.calls, result.FinalOutput
	observation.NewItems = projectImmediateItems(result.NewItems)
	observation.History = projectImmediateItems(result.FinalHistory)
	for _, response := range result.RawResponses {
		observation.AcceptedResponses = append(observation.AcceptedResponses, projectImmediateItems(response.Items))
	}
	return observation, nil
}

func generateImmediateSignal() ([]byte, error) {
	fixture := immediateSignalFixture{SchemaVersion: 1, SDKRevision: sdkRevision}
	for _, maxTurns := range []int{3, 1} {
		for _, streaming := range []bool{false, true} {
			c, err := runImmediateSignalCase(fmt.Sprintf("before_visible_max_turns_%d", maxTurns), streaming, maxTurns, "", false)
			if err != nil {
				return nil, err
			}
			fixture.Cases = append(fixture.Cases, c)
		}
	}
	for _, delta := range []struct{ name, event string }{{"text", "model.delta"}, {"reasoning", "model.reasoning_delta"}} {
		for _, boundary := range []struct {
			name     string
			maxTurns int
			tool     bool
		}{{"finalizer_max_turns_1", 1, false}, {"tool_boundary", 3, true}} {
			c, err := runImmediateSignalCase("after_visible_"+delta.name+"_"+boundary.name, true, boundary.maxTurns, delta.event, boundary.tool)
			if err != nil {
				return nil, err
			}
			fixture.Cases = append(fixture.Cases, c)
		}
	}
	b, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(b, '\n'), nil
}
