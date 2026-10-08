// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
)

func handoffCompactProjection(items []RunItem) []map[string]any {
	out := []map[string]any{}
	for _, item := range items {
		if item.Type == RunItemMessage {
			role := "user"
			if item.Agent != nil {
				role = "assistant"
			}
			out = append(out, map[string]any{"role": role, "text": item.Message.Text})
		}
	}
	return out
}
func handoffCompactInput(kind string) []RunItem {
	if kind == "empty" {
		return nil
	}
	items := []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "Original task: audit handoff context"}}}
	if kind == "short" {
		return items
	}
	for i := 0; i < 24; i++ {
		items = append(items, RunItem{Type: RunItemMessage, Agent: &Agent{Name: "assistant"}, Message: &MessageOutput{Text: fmt.Sprintf("step %d: ", i) + strings.Repeat("old context. ", 100)}})
	}
	return items
}
func TestNativeHandoffCompactionReference(t *testing.T) {
	cases := []map[string]any{}
	policies := []struct {
		name string
		cfg  HandoffHistoryConfig
	}{
		{"disabled", HandoffHistoryConfig{}},
		{"defaults", HandoffHistoryConfig{Enabled: true}},
		{"small", HandoffHistoryConfig{Enabled: true, MaxTokens: 300, TargetTokens: 120, PreserveRecentItems: 3, SummaryBulletLimit: 2}},
		{"equal-target", HandoffHistoryConfig{Enabled: true, MaxTokens: 300, TargetTokens: 300, PreserveRecentItems: 1}},
		{"one-token", HandoffHistoryConfig{Enabled: true, MaxTokens: 1}},
		{"preserve-all", HandoffHistoryConfig{Enabled: true, MaxTokens: 300, PreserveRecentItems: 100}},
	}
	for _, kind := range []string{"empty", "short", "long"} {
		for _, p := range policies {
			history, before, after, changed, reason := MaybeCompactHandoffInput(handoffCompactInput(kind), p.cfg)
			cases = append(cases, map[string]any{"kind": kind, "name": p.name, "policy": p.cfg, "history": handoffCompactProjection(history), "before": before, "after": after, "changed": changed, "reason": reason})
		}
	}
	runs := []map[string]any{}
	for _, streamed := range []bool{false, true} {
		for _, scenario := range []string{"disabled", "compact", "below", "ineffective", "carry", "filtered-empty"} {
			events := []string{}
			recordings := []map[string]any{}
			failures := []map[string]any{}
			target := &Agent{Name: "expert"}
			handoff := NewHandoff(target)
			handoff.OnHandoff = func(*RunContext, json.RawMessage) { events = append(events, "callback") }
			handoff.InputFilter = func(input, all []RunItem) []RunItem {
				events = append(events, "filter")
				if scenario == "filtered-empty" {
					return nil
				}
				out := []RunItem{}
				for _, item := range input {
					if item.Type == RunItemMessage {
						out = append(out, item)
					}
				}
				return out
			}
			source := &Agent{Name: "router", Handoffs: []*Handoff{handoff}}
			model := &subagentToolMockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "transfer"}}, {Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: handoff.ToolName, Input: json.RawMessage(`{}`)}}}}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}
			cfg := RunConfig{MaxTurns: 3, HandoffHistory: HandoffHistoryConfig{Enabled: scenario != "disabled", MaxTokens: 300, TargetTokens: 120, PreserveRecentItems: 3, SummaryBulletLimit: 2}}
			if scenario == "ineffective" {
				cfg.HandoffHistory.MaxTokens = 1
				cfg.HandoffHistory.TargetTokens = 0
			}
			cfg.CompactionRecorder = func(before, after int, summary string) {
				events = append(events, "compacted")
				recordings = append(recordings, map[string]any{"before": before, "after": after, "summary": summary})
			}
			cfg.CompactionFailureReporter = func(scope, reason string, before, after int) {
				events = append(events, "failed")
				failures = append(failures, map[string]any{"scope": scope, "reason": reason, "before": before, "after": after})
			}
			if scenario == "carry" {
				cfg.CompactionCarryForward = func(context.Context) string { events = append(events, "carry"); return "host state" }
			}
			input := handoffCompactInput("long")
			if scenario == "below" || scenario == "ineffective" {
				input = handoffCompactInput("short")
			}
			runner := NewRunnerWithModel(model)
			var result *RunResult
			var err error
			if streamed {
				r := runner.RunStreamed(context.Background(), source, input, cfg)
				for range r.Events {
				}
				result, err = r.FinalResult(), r.Err()
			} else {
				result, err = runner.Run(context.Background(), source, input, cfg)
			}
			if err != nil {
				t.Fatal(err)
			}
			if len(model.requests) != 2 {
				t.Fatalf("%s requests=%d", scenario, len(model.requests))
			}
			runs = append(runs, map[string]any{"scenario": scenario, "streamed": streamed, "events": events, "recordings": recordings, "failures": failures, "target_input": handoffCompactProjection(model.requests[1].Input), "final_history": handoffCompactProjection(result.FinalHistory), "new_items": handoffCompactProjection(result.NewItems), "last_agent": result.LastAgent.Name})
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases, "runs": runs}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
