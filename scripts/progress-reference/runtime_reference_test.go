// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"testing"
)

type progressStreamModel struct{ fail bool }

func (m *progressStreamModel) GetResponse(context.Context, ModelRequest) (*ModelResponse, error) {
	return nil, errors.New("oracle requires streaming interface")
}
func (m *progressStreamModel) StreamResponse(context.Context, ModelRequest) (*ModelStream, error) {
	response := &ModelResponse{
		Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "accepted text"}}},
		Usage: Usage{InputTokens: 13, OutputTokens: 7, CacheReadTokens: 5, CacheCreateTokens: 3},
	}
	events := make(chan ModelStreamEvent, 2)
	events <- ModelStreamEvent{Type: ModelStreamComplete, Response: response}
	if m.fail {
		events <- ModelStreamEvent{Type: ModelStreamError, Error: errors.New("after-complete failure")}
	}
	close(events)
	done := make(chan *ModelResponse, 1)
	done <- response
	close(done)
	return NewModelStream(events, done), nil
}
func (*progressStreamModel) GetRetryAdvice(error) *ModelRetryAdvice { return nil }
func (*progressStreamModel) CalculateCost(Usage) float64            { return 0.125 }
func (*progressStreamModel) Provider() string                       { return "progress-oracle" }

func TestNativeProgressRuntimeReference(t *testing.T) {
	cases := []any{}
	for _, streamed := range []bool{false, true} {
		for _, fail := range []bool{false, true} {
			for depth := 0; depth <= 2; depth++ {
				name := fmt.Sprintf("streamed_%t_failed_%t_depth_%d", streamed, fail, depth)
				t.Run(name, func(t *testing.T) {
					trackers := []*ProgressTracker{NewProgressTracker()}
					for i := 0; i < depth; i++ {
						trackers = append(trackers, NewChildTracker(trackers[i], fmt.Sprintf("task-%d", i)))
					}
					active := trackers[len(trackers)-1]
					runner := NewRunnerWithModel(&progressStreamModel{fail: fail})
					agent := &Agent{Name: "oracle", Model: "configured-label"}
					cfg := RunConfig{MaxTurns: 1, Hooks: NewPlatformHooks(active, NewEventStream(io.Discard))}
					var err error
					if streamed {
						result := runner.RunStreamed(context.Background(), agent, nil, cfg)
						for range result.Events {
						}
						result.FinalResult()
						err = result.Err()
					} else {
						_, err = runner.Run(context.Background(), agent, nil, cfg)
					}
					if (err != nil) != fail {
						t.Fatalf("unexpected error: %v", err)
					}
					observed := []any{}
					for _, tracker := range trackers {
						snap := tracker.Snapshot()
						observed = append(observed, map[string]any{
							"input": snap.InputTokens, "output": snap.OutputTokens,
							"cache_read": snap.CacheReadInputTokens, "cache_create": snap.CacheCreationInputTokens,
							"cost": snap.CostUsd, "models": snap.ModelUsage, "text": tracker.LastAssistantText(),
						})
					}
					cases = append(cases, map[string]any{"name": name, "streamed": streamed, "failed": fail, "depth": depth, "expected": observed})
				})
			}
		}
	}
	output, err := json.Marshal(map[string]any{"cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("PROGRESS_RUNTIME_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}
