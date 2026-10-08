// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"context"
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

type callbackAgentHooks struct {
	NoOpAgentHooks
	observe func(*RunContext, *Agent, *Agent)
}

func (h callbackAgentHooks) OnHandoff(ctx *RunContext, from, to *Agent) { h.observe(ctx, from, to) }

type callbackRunHooks struct {
	NoOpRunHooks
	observe func(*RunContext, *Agent, *Agent)
}

func (h callbackRunHooks) OnHandoff(ctx *RunContext, from, to *Agent) { h.observe(ctx, from, to) }

type callbackReferenceModel struct {
	*subagentToolMockModel
	observe func(ModelRequest)
}

func (m *callbackReferenceModel) GetResponse(ctx context.Context, req ModelRequest) (*ModelResponse, error) {
	m.observe(req)
	return m.subagentToolMockModel.GetResponse(ctx, req)
}
func (m *callbackReferenceModel) StreamResponse(ctx context.Context, req ModelRequest) (*ModelStream, error) {
	m.observe(req)
	return m.subagentToolMockModel.StreamResponse(ctx, req)
}

func callbackOutputs(items []RunItem) []map[string]any {
	outputs := []map[string]any{}
	for _, item := range items {
		if item.Type == RunItemToolOutput && item.ToolOutput != nil {
			outputs = append(outputs, map[string]any{"call_id": item.ToolOutput.CallID, "content": item.ToolOutput.Content, "is_error": item.ToolOutput.IsError})
		}
	}
	return outputs
}

func TestNativeHandoffCallbackReference(t *testing.T) {
	cases := []map[string]any{}
	for _, streamed := range []bool{false, true} {
		for _, scenario := range []struct{ name, raw string }{
			{"object", "{ \"reason\": \"billing\", \"nested\": {\"n\":1} }"},
			{"string", `"billing"`}, {"null", `null`}, {"array", `["billing",2,null,{"ok":true}]`},
			{"missing_arguments", ""}, {"nil_callback", `{"reason":"billing"}`},
			{"repeated_call", `{"first":true}`}, {"sibling_calls", `{"selected":true}`},
		} {
			mode := "normal"
			if streamed {
				mode = "streamed"
			}
			t.Run(mode+"/"+scenario.name, func(t *testing.T) {
				events := []string{}
				callbackInputs := []string{}
				contextsMatch, identitiesMatch := true, true
				var oldContext *RunContext
				source := &Agent{Name: "router", Model: "router-model"}
				target := &Agent{Name: "expert", Model: "expert-model", Instructions: "target original"}
				other := &Agent{Name: "other", Model: "other-model"}
				source.Hooks = callbackAgentHooks{observe: func(ctx *RunContext, from, to *Agent) {
					events = append(events, "old_agent_hook")
					oldContext = ctx
					identitiesMatch = identitiesMatch && from == source && to == target
				}}
				target.Hooks = callbackAgentHooks{observe: func(*RunContext, *Agent, *Agent) { events = append(events, "target_handoff_hook") }}
				runHooks := callbackRunHooks{observe: func(ctx *RunContext, from, to *Agent) {
					events = append(events, "run_hook")
					contextsMatch = contextsMatch && ctx == oldContext
					identitiesMatch = identitiesMatch && from == source && to == target
				}}
				h := NewHandoff(target)
				if scenario.name != "nil_callback" {
					h.OnHandoff = func(ctx *RunContext, input json.RawMessage) {
						events = append(events, "callback")
						callbackInputs = append(callbackInputs, string(input))
						contextsMatch = contextsMatch && ctx == oldContext
						target.Instructions = "target seeded by callback"
					}
				}
				filterOutputs := []map[string]any{}
				h.InputFilter = func(input, history []RunItem) []RunItem {
					events = append(events, "input_filter")
					filterOutputs = callbackOutputs(input)
					return input
				}
				source.Handoffs = []*Handoff{h, NewHandoff(other, WithOnHandoff(func(*RunContext, json.RawMessage) { events = append(events, "other_callback") }))}
				calls := []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: h.ToolName, Input: json.RawMessage(scenario.raw)}}}
				if scenario.name == "repeated_call" {
					calls = append(calls, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h2", Name: h.ToolName, Input: json.RawMessage(`{"second":true}`)}})
				}
				if scenario.name == "sibling_calls" {
					calls = append([]RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "lookup", Name: "lookup", Input: json.RawMessage(`{"unrelated":true}`)}}}, calls...)
					calls = append(calls, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "other", Name: "transfer_to_other", Input: json.RawMessage(`{"other":true}`)}})
				}
				model := &callbackReferenceModel{subagentToolMockModel: &subagentToolMockModel{responses: []*ModelResponse{
					{Items: calls}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}},
				}}, observe: func(req ModelRequest) { events = append(events, "model:"+req.Model) }}
				runner := NewRunnerWithModel(model)
				cfg := RunConfig{MaxTurns: 3, Hooks: runHooks}
				var result *RunResult
				var err error
				if streamed {
					stream := runner.RunStreamed(context.Background(), source, nil, cfg)
					for range stream.Events {
					}
					result, err = stream.FinalResult(), stream.Err()
				} else {
					result, err = runner.Run(context.Background(), source, nil, cfg)
				}
				if err != nil {
					t.Fatal(err)
				}
				wantEvents := []string{"model:router-model", "old_agent_hook", "run_hook"}
				wantInputs := []string{}
				wantInstructions := "target original"
				if scenario.name != "nil_callback" {
					wantEvents = append(wantEvents, "callback")
					wantInputs = append(wantInputs, scenario.raw)
					wantInstructions = "target seeded by callback"
				}
				wantEvents = append(wantEvents, "input_filter", "model:expert-model")
				if !reflect.DeepEqual(events, wantEvents) {
					t.Fatalf("events = %v, want %v", events, wantEvents)
				}
				if !reflect.DeepEqual(callbackInputs, wantInputs) {
					t.Fatalf("callback inputs = %q, want %q", callbackInputs, wantInputs)
				}
				if result == nil || result.LastAgent != target || result.FinalText() != "done" || !contextsMatch || !identitiesMatch {
					t.Fatalf("identity/context/result mismatch: %#v", result)
				}
				if len(model.requests) != 2 {
					t.Fatalf("requests = %d", len(model.requests))
				}
				request := model.requests[1]
				if request.Instructions != wantInstructions {
					t.Fatalf("target instructions = %q", request.Instructions)
				}
				outputs := callbackOutputs(request.Input)
				if len(outputs) != len(calls) || !reflect.DeepEqual(outputs, filterOutputs) {
					t.Fatalf("outputs differ between filter and target: %v, %v", filterOutputs, outputs)
				}
				for i, output := range outputs {
					wantError := calls[i].ToolCall.ID != "h1"
					wantContent := "Handing off to expert"
					if wantError {
						wantContent = "not executed: the conversation was handed off to expert in this turn"
					}
					if output["call_id"] != calls[i].ToolCall.ID || output["is_error"] != wantError || output["content"] != wantContent {
						t.Fatalf("output = %v", output)
					}
				}
				cases = append(cases, map[string]any{
					"mode": mode, "scenario": scenario.name, "input_raw": scenario.raw,
					"has_callback": h.OnHandoff != nil, "has_custom_schema": h.InputType != nil,
					"callback_inputs_raw": callbackInputs, "callback_count": len(callbackInputs), "events": events,
					"same_run_context": contextsMatch, "hook_agent_identities_match": identitiesMatch,
					"last_agent_is_target": result.LastAgent == target, "last_agent_name": result.LastAgent.Name,
					"target_model": request.Model, "target_instructions": request.Instructions,
					"filter_outputs": filterOutputs, "target_input_outputs": outputs, "final_text": result.FinalText(),
				})
			})
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases, "source_observations": []map[string]string{{
		"source":      "internal/agent/runner.go:1511-1518",
		"observation": "When InputType is set, validation failure only logs a warning and still invokes OnHandoff; these executable callback cases intentionally use no custom schema.",
	}}}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
