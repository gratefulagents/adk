// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

type handoffTraceProbe struct {
	NoOpTracingProcessor
	events       []map[string]any
	open         bool
	starts, ends int
	ended        bool
}

func (p *handoffTraceProbe) stage(name string) {
	p.events = append(p.events, map[string]any{"name": name, "open": p.open})
}
func (p *handoffTraceProbe) OnSpanStart(s *Span) {
	if s.Name == "handoff" {
		p.open = true
		p.starts++
		p.stage("span_start")
	}
}
func (p *handoffTraceProbe) OnSpanEnd(s *Span) {
	if s.Name == "handoff" {
		p.open = false
		p.ends++
		p.ended = !s.EndTime.IsZero()
		p.stage("span_end")
	}
}
func (p *handoffTraceProbe) OnTraceEnd(*Trace) { p.stage("trace_end") }

type handoffTraceAgentHook struct {
	NoOpAgentHooks
	p *handoffTraceProbe
}

func (h handoffTraceAgentHook) OnHandoff(*RunContext, *Agent, *Agent) { h.p.stage("agent_hook") }

type handoffTraceRunHook struct {
	NoOpRunHooks
	p *handoffTraceProbe
}

func (h handoffTraceRunHook) OnHandoff(*RunContext, *Agent, *Agent) { h.p.stage("run_hook") }
func TestNativeHandoffTracingReference(t *testing.T) {
	cases := []map[string]any{}
	for _, streamed := range []bool{false, true} {
		for _, scenario := range []string{"plain", "filtered", "carry", "cancel"} {
			p := &handoffTraceProbe{events: []map[string]any{}}
			ctx, cancel := context.WithCancel(context.Background())
			target := &Agent{Name: "target", InstructionsFn: func(*RunContext, *Agent) string { p.stage("target"); return "target" }}
			handoff := NewHandoff(target)
			handoff.OnHandoff = func(*RunContext, json.RawMessage) {
				p.stage("callback")
				if scenario == "cancel" {
					cancel()
				}
			}
			if scenario == "filtered" || scenario == "carry" {
				handoff.InputFilter = func(items, all []RunItem) []RunItem {
					p.stage("filter")
					out := []RunItem{}
					for _, item := range items {
						if item.Type == RunItemMessage {
							out = append(out, item)
						}
					}
					return out
				}
			}
			source := &Agent{Name: "source", Handoffs: []*Handoff{handoff}, Hooks: handoffTraceAgentHook{p: p}}
			model := &subagentToolMockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: handoff.ToolName, Input: json.RawMessage(`{}`)}}}}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}
			cfg := RunConfig{MaxTurns: 3, TracingProcessor: p, Hooks: handoffTraceRunHook{p: p}}
			input := []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "original task"}}}
			if scenario == "carry" {
				for i := 0; i < 12; i++ {
					input = append(input, RunItem{Type: RunItemMessage, Agent: &Agent{Name: "assistant"}, Message: &MessageOutput{Text: strings.Repeat("older context. ", 100)}})
				}
				cfg.HandoffHistory = HandoffHistoryConfig{Enabled: true, MaxTokens: 300, TargetTokens: 120}
				cfg.CompactionCarryForward = func(context.Context) string { p.stage("carry"); return "host state" }
			}
			runner := NewRunnerWithModel(model)
			var err error
			if streamed {
				r := runner.RunStreamed(ctx, source, input, cfg)
				for range r.Events {
				}
				err = r.Err()
			} else {
				_, err = runner.Run(ctx, source, input, cfg)
			}
			cancel()
			if (err != nil) != (scenario == "cancel") {
				t.Fatalf("%s: %v", scenario, err)
			}
			cases = append(cases, map[string]any{"scenario": scenario, "streamed": streamed, "events": p.events, "starts": p.starts, "ends": p.ends, "end_time_set": p.ended, "error": err != nil})
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
