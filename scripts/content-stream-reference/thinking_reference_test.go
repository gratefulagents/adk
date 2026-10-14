// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"strings"
	"testing"
	"time"
)

func TestNativeThinkingDeltaReference(t *testing.T) {
	type action struct {
		AfterNanos int64  `json:"afterNanos"`
		Text       string `json:"text"`
		Flush      bool   `json:"flush"`
	}
	cases := []struct {
		name    string
		actions []action
	}{
		{"empty", []action{{}, {Flush: true}, {Flush: true}}},
		{"empty-flush-delays-first", []action{{Flush: true}, {Text: "first"}, {AfterNanos: 500000000, Text: "next"}, {Flush: true}}},
		{"time-boundaries", []action{{Text: "α"}, {AfterNanos: 499999999, Text: "beta"}, {AfterNanos: 500000000, Text: "γ"}, {AfterNanos: 500000001, Text: "tail"}, {Flush: true}, {Flush: true}}},
		{"byte-boundaries", []action{{Text: "first"}, {Text: strings.Repeat("a", 4095)}, {Text: "b"}, {Text: strings.Repeat("😀", 1023)}, {Text: "😀"}, {Text: "last"}, {Flush: true}}},
		{"oversized", []action{{Text: strings.Repeat("😀", 2000)}, {Text: " "}, {Text: "\n"}, {Flush: true}}},
		{"flush-resets-clock", []action{{Text: "first"}, {AfterNanos: 400000000, Text: "buffer"}, {AfterNanos: 400000000, Flush: true}, {AfterNanos: 500000000, Text: "not-yet"}, {AfterNanos: 900000000, Text: "now"}, {Flush: true}}},
	}
	results := []map[string]any{}
	for _, c := range cases {
		var output bytes.Buffer
		stream := NewEventStream(&output)
		emitter := newThinkingDeltaEmitter(stream, "attempt", "worker")
		clock := time.Unix(0, 0)
		emitter.now = func() time.Time { return clock }
		observations := []map[string]any{}
		for _, a := range c.actions {
			clock = time.Unix(0, a.AfterNanos)
			if a.Flush {
				emitter.Flush()
			} else {
				emitter.Chunk(a.Text)
			}
			chunks := []string{}
			for _, line := range bytes.Split(bytes.TrimSpace(output.Bytes()), []byte("\n")) {
				if len(line) == 0 {
					continue
				}
				var event ContentEvent
				if err := json.Unmarshal(line, &event); err != nil {
					t.Fatal(err)
				}
				if event.Type != "assistant_thinking_delta" || event.ToolUseID != "attempt" || event.AgentName != "worker" {
					t.Fatalf("bad correlation: %+v", event)
				}
				chunks = append(chunks, event.Message)
			}
			observations = append(observations, map[string]any{"action": a, "chunks": chunks})
			output.Reset()
		}
		results = append(results, map[string]any{"name": c.name, "observations": observations})
	}
	data, err := json.Marshal(results)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("THINKING_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}

func TestNativeThinkingAttemptReference(t *testing.T) {
	cases := []map[string]any{}
	for _, failed := range []bool{false, true} {
		model := &reasoningSinkModel{mockModel: &mockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemReasoning, Reasoning: &ReasoningData{Text: "reason"}}, {Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}, Usage: Usage{InputTokens: 5, OutputTokens: 2, CacheReadTokens: 3, CacheCreateTokens: 1}}}}, chunks: []string{"reason"}}
		if failed {
			model.errors = []error{errors.New("failure")}
		}
		var output bytes.Buffer
		hooks := NewPlatformHooks(NewProgressTracker(), NewEventStream(&output))
		_, err := NewRunnerWithModel(model).Run(context.Background(), &Agent{Name: "worker", Model: "mock/model"}, nil, RunConfig{Hooks: hooks, MaxTurns: 2})
		if (err != nil) != failed {
			t.Fatalf("unexpected run result: %v", err)
		}
		events := []ContentEvent{}
		for _, event := range decodeContentEvents(t, &output) {
			if event.Type != "llm_attempt" {
				continue
			}
			if event.ToolUseID == "" {
				t.Fatal("missing attempt identity")
			}
			event.ToolUseID = "attempt"
			event.Timestamp = time.Time{}
			event.AttemptLatencyMs = 0
			events = append(events, event)
		}
		cases = append(cases, map[string]any{"failed": failed, "events": events})
	}
	data, err := json.Marshal(cases)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("THINKING_ATTEMPT_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
