package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"testing"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestImmediateSignalRunnerProjection(t *testing.T) {
	first, err := generateImmediateSignal()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generateImmediateSignal()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatal("immediate-signal projection is not deterministic")
	}
	golden, err := os.ReadFile("../../fixtures/handoff/sdk-immediate-signal.json")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, golden) {
		t.Fatal("immediate-signal fixture is stale")
	}
	if bytes.Contains(first, []byte("hidden-superseded-answer")) {
		t.Fatal("canceled provider response leaked into requests, accepted responses, history, or consumed SDK events")
	}
	var fixture immediateSignalFixture
	if err := json.Unmarshal(first, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.SchemaVersion != 1 || fixture.SDKRevision != sdkRevision || len(fixture.Cases) != 8 {
		t.Fatalf("unexpected fixture metadata: version=%d revision=%s cases=%d", fixture.SchemaVersion, fixture.SDKRevision, len(fixture.Cases))
	}
	cases := []struct {
		name      string
		streaming bool
		maxTurns  int
		visible   string
		tool      bool
	}{
		{"before_visible_max_turns_3", false, 3, "", false},
		{"before_visible_max_turns_3", true, 3, "", false},
		{"before_visible_max_turns_1", false, 1, "", false},
		{"before_visible_max_turns_1", true, 1, "", false},
		{"after_visible_text_finalizer_max_turns_1", true, 1, "model.delta", false},
		{"after_visible_text_tool_boundary", true, 3, "model.delta", true},
		{"after_visible_reasoning_finalizer_max_turns_1", true, 1, "model.reasoning_delta", false},
		{"after_visible_reasoning_tool_boundary", true, 3, "model.reasoning_delta", true},
	}
	agent := "oracle"
	initial := immediateItem{Type: "message", Message: "initial"}
	steer1 := immediateItem{Type: "message", Message: "steer-1"}
	steer2 := immediateItem{Type: "message", Message: "steer-2"}
	answer1 := immediateItem{Type: "message", Agent: &agent, Message: "answer-1"}
	answer2 := immediateItem{Type: "message", Agent: &agent, Message: "answer-2"}
	call := immediateItem{Type: "tool_call", Agent: &agent, ToolCall: &sdk.ToolCallData{ID: "call-1", Name: "inspect", Input: json.RawMessage(`{}`)}}
	output := immediateItem{Type: "tool_output", Agent: &agent, ToolOutput: &sdk.ToolOutputData{CallID: "call-1", Content: "tool result"}}
	wrappedOutput := immediateItem{Type: "tool_output", Agent: &agent, ToolOutput: &sdk.ToolOutputData{CallID: "call-1", Content: "BEGIN UNTRUSTED TOOL OUTPUT\ntool result\nEND UNTRUSTED TOOL OUTPUT"}}
	for i, scenario := range cases {
		t.Run(scenario.name+map[bool]string{false: "/complete", true: "/stream"}[scenario.streaming], func(t *testing.T) {
			want := immediateSignalCase{
				Name: scenario.name, Streaming: scenario.streaming, MaxTurns: scenario.maxTurns,
				SignalAfter: "pending_model_attempt", RequestCount: 2,
				Requests: []immediateRequest{
					{Input: []immediateItem{initial}},
					{Input: []immediateItem{initial, steer1, steer2}},
				},
				FirstAttemptCanceled: true, RetryAdviceCalls: 1,
				PollCalls: 2, FinalizerCalls: 1,
				Callbacks: []immediateSignalCallback{
					{Name: "poll", Items: []immediateItem{}},
					{Name: "poll", Items: []immediateItem{steer1, steer2}},
					{Name: "finalizer", Items: []immediateItem{}},
				},
				Output: "answer-2", Error: "none",
				AcceptedResponses:   [][]immediateItem{{answer2}},
				NewItems:            []immediateItem{steer1, steer2, answer2},
				ConsumedStreamItems: []immediateItem{}, ConsumedStreamDeltas: []immediateSignalDelta{},
			}
			if scenario.visible != "" {
				want.SignalAfter = "host_received_" + scenario.visible
				want.FirstAttemptCanceled, want.RetryAdviceCalls = false, 0
				want.ConsumedStreamDeltas = []immediateSignalDelta{{Name: scenario.visible, Text: "visible-first-attempt"}}
				if scenario.tool {
					want.ToolCalls = 1
					want.Requests[1].Input = []immediateItem{initial, call, wrappedOutput, steer1, steer2}
					want.AcceptedResponses = [][]immediateItem{{call}, {answer2}}
					want.NewItems = []immediateItem{call, output, steer1, steer2, answer2}
				} else {
					want.FinalizerCalls = 2
					want.Requests[1].Input = []immediateItem{initial, answer1, steer1, steer2}
					want.AcceptedResponses = [][]immediateItem{{answer1}, {answer2}}
					want.NewItems = []immediateItem{answer1, steer1, steer2, answer2}
					want.Callbacks = []immediateSignalCallback{
						{Name: "poll", Items: []immediateItem{}},
						{Name: "finalizer", Items: []immediateItem{steer1, steer2}},
						{Name: "poll", Items: []immediateItem{}},
						{Name: "finalizer", Items: []immediateItem{}},
					}
				}
			}
			want.History = append([]immediateItem{initial}, want.NewItems...)
			if scenario.tool {
				want.History[2] = wrappedOutput
			}
			if scenario.streaming {
				want.ConsumedStreamItems = want.NewItems
			}
			if !reflect.DeepEqual(fixture.Cases[i], want) {
				actual, _ := json.Marshal(fixture.Cases[i])
				expected, _ := json.Marshal(want)
				t.Fatalf("got %s\nwant %s", actual, expected)
			}
		})
	}
}
