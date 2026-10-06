// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
	"os"
	"strings"
	"testing"
)

type instructionModel struct {
	blockingModel
	requests     []agentsdk.ModelRequest
	continueOnce bool
	failOnce     bool
}

type instructionProvider struct{ model *instructionModel }

func (p *instructionProvider) GetModel(string) (agentsdk.Model, error) { return p.model, nil }
func (p *instructionProvider) Close() error                            { return nil }

func (m *instructionModel) GetResponse(_ context.Context, req agentsdk.ModelRequest) (*agentsdk.ModelResponse, error) {
	m.requests = append(m.requests, req)
	if m.failOnce && len(m.requests) == 1 {
		return nil, fmt.Errorf("overloaded")
	}
	response := &agentsdk.ModelResponse{Items: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: "done"}}}}
	if m.continueOnce && len(m.requests) == 1 {
		end := false
		response.EndTurn = &end
	}
	return response, nil
}
func (m *instructionModel) GetRetryAdvice(error) *agentsdk.ModelRetryAdvice {
	if m.failOnce {
		return &agentsdk.ModelRetryAdvice{ShouldRetry: true, Reason: "overloaded"}
	}
	return nil
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
	for index, model := range []string{"gpt-5-mini", "gpt-6", "gpt-5.4", "unknown"} {
		for _, custom := range []bool{false, true} {
			for _, streaming := range []bool{false, true} {
				for _, blankCarry := range []bool{false, true} {
					config := Config{Model: model, Provider: []string{"", "anthropic", "copilot", "local"}[index], Features: &Features{Runtime: RuntimeFeatures{Compaction: true}}}
					if blankCarry {
						config.CompactionCarryForward = func(context.Context) string { return "" }
					}
					modeName, displayName := "", ""
					if index > 0 {
						modeName = []string{"", "planner", "unicode", "fallback"}[index]
						displayName = []string{"", " Planning ", "Δ", ""}[index]
						config.ModeSnapshot = &sdkmode.TemplateSpec{Name: modeName, DisplayName: displayName}
					}
					if custom {
						config.WorkingStateText = " custom state \nΔ "
						config.CompactionConfig = &agentsdk.CompactionConfig{Enabled: true, TriggerTokens: 90000, TargetTokens: 40000, PreserveRecentItems: 3, PreserveInitialUserMessages: 1, SummaryBulletLimit: 7}
						config.CompactionModelResolver = func(context.Context, string) (int, int, bool) { return 0, 0, false }
					}
					cfg := BuildRunConfig(config, nil)
					cfg.MaxTurns = 1
					texts := observeCompaction(t, model, cfg, streaming, 2000)
					compactions = append(compactions, map[string]any{"model": model, "provider": config.Provider, "mode_name": modeName, "mode_display_name": displayName, "blank_carry": blankCarry, "custom": custom, "streaming": streaming, "policy": cfg.CompactionConfig, "working_state_text": config.WorkingStateText, "text_sha256": texts})
				}
			}
		}
	}
	for _, enabled := range []bool{false, true} {
		for _, override := range []int{-1, 0, 1, 2} {
			for _, streaming := range []bool{false, true} {
				config := Config{Model: "gpt-5-mini", Features: &Features{Runtime: RuntimeFeatures{Compaction: enabled}}}
				var explicit any
				if override >= 0 {
					explicit = override > 0
					config.CompactionConfig = &agentsdk.CompactionConfig{Enabled: override > 0, TriggerTokens: 90000, TargetTokens: 40000, PreserveRecentItems: 3, PreserveInitialUserMessages: 1, SummaryBulletLimit: 7}
				}
				repeat := 2000
				if override == 2 {
					config.CompactionConfig = &agentsdk.CompactionConfig{Enabled: true, TriggerTokens: 180000, TargetTokens: 100000, PreserveRecentItems: 12, PreserveInitialUserMessages: 2, SummaryBulletLimit: 4}
					repeat = 400
				}
				cfg := BuildRunConfig(config, nil)
				cfg.MaxTurns = 1
				texts := observeCompaction(t, config.Model, cfg, streaming, repeat)
				compactions = append(compactions, map[string]any{"model": config.Model, "provider": "", "mode_name": "", "mode_display_name": "", "blank_carry": false, "custom": false, "streaming": streaming, "policy": cfg.CompactionConfig, "working_state_text": "", "feature_enabled": enabled, "explicit_policy": explicit, "default_policy": override == 2, "input_repeat": repeat, "text_sha256": texts})
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
	resolvers := []map[string]any{}
	for _, feature := range []bool{false, true} {
		for _, thresholds := range [][3]int{{0, 0, 0}, {0, 10, 1}, {300000, 150000, 1}, {90000, 40000, 1}, {90000, 0, 1}, {90000, 180000, 1}} {
			for _, streaming := range []bool{false, true} {
				for _, continuing := range []bool{false, true} {
					calls := []string{}
					config := Config{Model: "gpt-5-mini", Features: &Features{Runtime: RuntimeFeatures{Compaction: feature}},
						CompactionConfig: &agentsdk.CompactionConfig{Enabled: true, TriggerTokens: 180000, TargetTokens: 100000, PreserveRecentItems: 12, PreserveInitialUserMessages: 2, SummaryBulletLimit: 4},
						CompactionModelResolver: func(_ context.Context, model string) (int, int, bool) {
							calls = append(calls, model)
							return thresholds[0], thresholds[1], thresholds[2] == 1
						},
					}
					cfg := BuildRunConfig(config, nil)
					cfg.MaxTurns = 3
					m := &instructionModel{continueOnce: continuing}
					r := agentsdk.NewRunnerWithModel(m)
					a := &agentsdk.Agent{Name: "agent", Model: config.Model, Instructions: "base"}
					input := []agentsdk.RunItem{}
					for i := 0; i < 100; i++ {
						repeat := 400
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
							t.Fatalf("resolver run: %v", err)
						}
					}
					digests := []string{}
					for _, req := range m.requests {
						h := sha256.New()
						for _, item := range req.Input {
							if item.Message == nil {
								t.Fatal("unexpected nonmessage")
							}
							fmt.Fprintf(h, "%d:%s", len(item.Message.Text), item.Message.Text)
						}
						digests = append(digests, fmt.Sprintf("%x", h.Sum(nil)))
					}
					resolvers = append(resolvers, map[string]any{"feature": feature, "thresholds": thresholds, "streaming": streaming, "continuing": continuing, "models": calls, "text_digests": digests})
				}
			}
		}
	}
	for _, fallback := range []bool{false, true} {
		for _, streaming := range []bool{false, true} {
			calls := []string{}
			cfg := agentsdk.RunConfig{MaxTurns: 3, CompactionConfig: agentsdk.CompactionConfig{Enabled: true, TriggerTokens: 180000, TargetTokens: 100000}, CompactionModelResolver: func(_ context.Context, model string) (int, int, bool) {
				calls = append(calls, model)
				return 0, 0, false
			}}
			if fallback {
				cfg.FallbackModels = []string{"gpt-6"}
			}
			m := &instructionModel{failOnce: true}
			r := agentsdk.NewRunnerWithModel(m)
			if fallback {
				r = agentsdk.NewRunnerWithProvider(&instructionProvider{model: m})
			}
			a := &agentsdk.Agent{Name: "agent", Model: "gpt-5-mini"}
			if streaming {
				s := r.RunStreamed(context.Background(), a, nil, cfg)
				for range s.Events {
				}
				if s.FinalResult().FinalText() != "done" {
					t.Fatal("missing retry streamed answer")
				}
			} else {
				out, err := r.Run(context.Background(), a, nil, cfg)
				if err != nil || out.FinalText() != "done" {
					t.Fatalf("retry run: %v", err)
				}
			}
			resolvers = append(resolvers, map[string]any{"retry": true, "fallback": fallback, "streaming": streaming, "models": calls})
		}
	}
	data, err = json.Marshal(resolvers)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("RUN_RESOLVER_OUTPUT"), data, 0600); err != nil {
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

func observeCompaction(t *testing.T, model string, cfg agentsdk.RunConfig, streaming bool, inputRepeat int) []string {
	t.Helper()
	m := &instructionModel{}
	r := agentsdk.NewRunnerWithModel(m)
	a := &agentsdk.Agent{Name: "agent", Model: model, Instructions: "base"}
	input := []agentsdk.RunItem{}
	for i := 0; i < 100; i++ {
		repeat := inputRepeat
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
	return texts
}
