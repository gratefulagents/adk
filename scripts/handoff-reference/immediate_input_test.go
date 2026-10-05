package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"testing"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestImmediateInputRunnerProjection(t *testing.T) {
	first, err := generateImmediateInput()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generateImmediateInput()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatal("immediate-input projection is not deterministic")
	}
	golden, err := os.ReadFile("../../fixtures/handoff/sdk-immediate-input.json")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, golden) {
		t.Fatal("immediate-input fixture is stale")
	}
	var fixture immediateFixture
	if err := json.Unmarshal(first, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.SchemaVersion != 1 || fixture.SDKRevision != sdkRevision || len(fixture.Cases) != 14 {
		t.Fatalf("unexpected fixture metadata: version=%d revision=%s cases=%d", fixture.SchemaVersion, fixture.SDKRevision, len(fixture.Cases))
	}

	agent := "oracle"
	initial := immediateItem{Type: "message", Message: "initial"}
	polled1 := immediateItem{Type: "message", Message: "polled-1"}
	polled2 := immediateItem{Type: "message", Message: "polled-2"}
	late := immediateItem{Type: "message", Message: "late"}
	answer1 := immediateItem{Type: "message", Agent: &agent, Message: "answer-1"}
	answer2 := immediateItem{Type: "message", Agent: &agent, Message: "answer-2"}
	call := immediateItem{Type: "tool_call", Agent: &agent, ToolCall: &sdk.ToolCallData{ID: "call-1", Name: "inspect", Input: json.RawMessage(`{}`)}}
	output := immediateItem{Type: "tool_output", Agent: &agent, ToolOutput: &sdk.ToolOutputData{CallID: "call-1", Content: "tool result"}}
	wrappedOutput := immediateItem{Type: "tool_output", Agent: &agent, ToolOutput: &sdk.ToolOutputData{CallID: "call-1", Content: "BEGIN UNTRUSTED TOOL OUTPUT\ntool result\nEND UNTRUSTED TOOL OUTPUT"}}
	for i, c := range fixture.Cases {
		t.Run(c.Name+map[bool]string{false: "/normal", true: "/stream"}[c.Streaming], func(t *testing.T) {
			want := immediateCase{
				Name: c.Name, Streaming: c.Streaming, MaxTurns: 3,
				Requests:  []immediateRequest{{Input: []immediateItem{initial}}},
				PollCalls: 1, FinalizerCalls: 1, Output: "answer-1", Error: "none",
				NewItems: []immediateItem{answer1},
			}
			switch c.Name {
			case "poll_before_first_request":
				want.Requests[0].Input = []immediateItem{initial, polled1, polled2}
				want.NewItems = []immediateItem{polled1, polled2, answer1}
			case "poll_after_tool_response":
				want.Requests = append(want.Requests, immediateRequest{Input: []immediateItem{initial, call, wrappedOutput, polled1, polled2}})
				want.PollCalls, want.ToolCalls, want.Output = 2, 1, "answer-2"
				want.NewItems = []immediateItem{call, output, polled1, polled2, answer2}
			case "poll_error_best_effort", "empty_finalizer_ends":
			case "finalizer_late_input":
				want.Requests = append(want.Requests, immediateRequest{Input: []immediateItem{initial, answer1, late}})
				want.PollCalls, want.FinalizerCalls, want.Output = 2, 2, "answer-2"
				want.NewItems = []immediateItem{answer1, late, answer2}
			case "finalizer_error_propagated":
				want.Output, want.Error = nil, "finalizer_error"
			case "max_turns_finalizer_extends":
				want.MaxTurns = 1
				want.Requests = append(want.Requests, immediateRequest{Input: []immediateItem{initial, call, wrappedOutput, late}})
				want.PollCalls, want.FinalizerCalls, want.ToolCalls, want.Output = 2, 2, 1, "answer-2"
				want.NewItems = []immediateItem{call, output, late, answer2}
			default:
				t.Fatalf("unexpected case %q", c.Name)
			}
			want.History = append([]immediateItem{initial}, want.NewItems...)
			if want.ToolCalls > 0 {
				want.History[2] = wrappedOutput
			}
			if !reflect.DeepEqual(c, want) {
				actual, _ := json.Marshal(c)
				expected, _ := json.Marshal(want)
				t.Fatalf("got %s\nwant %s", actual, expected)
			}
			if c.Streaming != (i%2 == 1) {
				t.Fatal("expected adjacent normal/stream pairs")
			}
			if c.Streaming {
				c.Streaming = false
				if !reflect.DeepEqual(c, fixture.Cases[i-1]) {
					t.Fatal("normal and streamed projections differ")
				}
			}
		})
	}
}
