// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
	"context"
	"encoding/json"
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"os"
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
		config := Config{}
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
			cases = append(cases, observeInstructions(t, "base", cfg, streaming, map[string]any{"base": "base", "extra": cfg.AdditionalInstructions, "builder": true, "streaming": streaming, "feature_summary": config.FeatureSummary, "mode_directive_text": config.ModeDirectiveText, "final_check_instructions": config.FinalCheckInstructions}))
		}
	}
	data, err := json.Marshal(cases)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("RUN_INSTRUCTIONS_OUTPUT"), data, 0600); err != nil {
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
