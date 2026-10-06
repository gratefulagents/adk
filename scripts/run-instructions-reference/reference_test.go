// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
	"strings"
	"testing"
)

type instructionModel struct {
	blockingModel
	requests []agentsdk.ModelRequest
}

func (m *instructionModel) GetResponse(_ context.Context, req agentsdk.ModelRequest) (*agentsdk.ModelResponse, error) {
	m.requests = append(m.requests, req)
	return &agentsdk.ModelResponse{Items: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "done"}}}}, nil
}
func (m *instructionModel) StreamResponse(ctx context.Context, req agentsdk.ModelRequest) (*agentsdk.ModelStream, error) {
	response, err := m.GetResponse(ctx, req)
	if err != nil {
		return nil, err
	}
	events := make(chan agentsdk.ModelStreamEvent, 1)
	done := make(chan *agentsdk.ModelResponse, 1)
	events <- agentsdk.ModelStreamEvent{Type: agentsdk.ModelStreamComplete, Response: response}
	close(events)
	done <- response
	close(done)
	return agentsdk.NewModelStream(events, done), nil
}
func TestRunInstructionsReference(t *testing.T) {
	cases := []map[string]any{}
	for _, base := range []string{"", " \n\t", " base\n "} {
		for _, extra := range []string{"", " \t", " extra ", "Δ\nnext"} {
			for _, streaming := range []bool{false, true} {
				cfg := agentsdk.RunConfig{AdditionalInstructions: extra, MaxTurns: 1}
				cases = append(cases, observeInstructions(t, base, cfg, streaming, map[string]any{"base": base, "extra": extra, "builder": false, "streaming": streaming}))
			}
		}
	}
	for mask := 0; mask < 8; mask++ {
		config := Config{WorkingStateText: []string{"", " \n\t", " host state ", "Δ\nnext"}[mask%4]}
		if mask&1 != 0 {
			config.FeatureSummary = "  tools enabled \n"
		}
		if mask&2 != 0 {
			config.ModeDirectiveText = " \nmode directive \t"
		}
		if mask&4 != 0 {
			config.FinalCheckInstructions = " final check \n"
		}
		for _, streaming := range []bool{false, true} {
			cfg := BuildRunConfig(config, nil)
			cfg.MaxTurns = 1
			cases = append(cases, observeInstructions(t, "base", cfg, streaming, map[string]any{"base": "base", "extra": cfg.AdditionalInstructions, "builder": true, "streaming": streaming, "feature_summary": config.FeatureSummary, "mode_directive_text": config.ModeDirectiveText, "final_check_instructions": config.FinalCheckInstructions, "working_state_text": config.WorkingStateText, "working_state_context": cfg.WorkingStateContext}))
		}
	}
	data, err := json.Marshal(cases)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("RUN_INSTRUCTIONS_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
	compactions := []map[string]any{}
	for _, model := range []string{"gpt-5-mini", "gpt-6", "gpt-5.4", "unknown"} {
		for _, custom := range []bool{false, true} {
			for _, streaming := range []bool{false, true} {
				config := Config{Model: model, Features: &Features{Runtime: RuntimeFeatures{Compaction: true}}, CompactionCarryForward: func(context.Context) string { return "" }}
				if custom {
					config.CompactionConfig = &agentsdk.CompactionConfig{Enabled: true, TriggerTokens: 90000, TargetTokens: 40000, PreserveRecentItems: 3, PreserveInitialUserMessages: 1, SummaryBulletLimit: 7}
					config.CompactionModelResolver = func(context.Context, string) (int, int, bool) { return 0, 0, false }
				}
				cfg := BuildRunConfig(config, nil)
				cfg.MaxTurns = 1
				m := &instructionModel{}
				r := agentsdk.NewRunnerWithModel(m)
				a := &agentsdk.Agent{Name: "agent", Model: model, Instructions: "base"}
				input := []agentsdk.RunItem{}
				for i := 0; i < 100; i++ {
					repeat := 2000
					if i >= 80 {
						repeat = 2
					}
					input = append(input, agentsdk.RunItem{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: fmt.Sprintf("message %03d: ", i) + strings.Repeat("old conversation ", repeat)}})
				}
				if streaming {
					s := r.RunStreamed(context.Background(), a, input, cfg)
					for range s.Events {
					}
					if s.FinalResult().FinalText() != "done" {
						t.Fatal("missing streamed answer")
					}
				} else {
					out, err := r.Run(context.Background(), a, input, cfg)
					if err != nil || out.FinalText() != "done" {
						t.Fatalf("compaction run: %v", err)
					}
				}
				if len(m.requests) != 1 {
					t.Fatal("unexpected compaction requests")
				}
				texts := []string{}
				for _, item := range m.requests[0].Input {
					if item.Message == nil {
						t.Fatal("unexpected nonmessage")
					}
					texts = append(texts, fmt.Sprintf("%x", sha256.Sum256([]byte(item.Message.Text))))
				}
				compactions = append(compactions, map[string]any{"model": model, "custom": custom, "streaming": streaming, "policy": cfg.CompactionConfig, "text_sha256": texts})
			}
		}
	}
	data, err = json.Marshal(compactions)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("RUN_COMPACTION_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
func observeInstructions(t *testing.T, base string, cfg agentsdk.RunConfig, streaming bool, result map[string]any) map[string]any {
	t.Helper()
	m := &instructionModel{}
	r := agentsdk.NewRunnerWithModel(m)
	a := &agentsdk.Agent{Name: "test", Instructions: base}
	if streaming {
		s := r.RunStreamed(context.Background(), a, nil, cfg)
		for range s.Events {
		}
		if s.FinalResult().FinalText() != "done" {
			t.Fatal("missing streamed answer")
		}
	} else {
		out, err := r.Run(context.Background(), a, nil, cfg)
		if err != nil {
			t.Fatal(err)
		}
		if out.FinalText() != "done" {
			t.Fatal("missing answer")
		}
	}
	if len(m.requests) != 1 {
		t.Fatalf("requests: %d", len(m.requests))
	}
	result["instructions"] = m.requests[0].Instructions
	return result
}
