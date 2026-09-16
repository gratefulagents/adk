// GPL-3.0-only. Executes the pinned SDK runner; see runner_README.md.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sync"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type object = map[string]any

type item struct {
	Type      string          `json:"type"`
	Text      string          `json:"text,omitempty"`
	ID        string          `json:"id,omitempty"`
	Name      string          `json:"name,omitempty"`
	Arguments json.RawMessage `json:"arguments,omitempty"`
	Content   string          `json:"content,omitempty"`
	IsError   bool            `json:"is_error,omitempty"`
}
type response struct {
	Error        string   `json:"error,omitempty"`
	Items        []item   `json:"items"`
	EndTurn      *bool    `json:"end_turn"`
	InputTokens  int64    `json:"input_tokens"`
	OutputTokens int64    `json:"output_tokens"`
	Deltas       []string `json:"deltas"`
}
type scenario struct {
	Deny         bool            `json:"deny"`
	Resume       bool            `json:"resume"`
	ChatLoop     bool            `json:"chat_loop"`
	Untrusted    bool            `json:"untrusted"`
	OutputCap    int             `json:"output_cap"`
	SchemaName   string          `json:"schema_name"`
	SchemaStrict *bool           `json:"schema_strict"`
	CustomParser bool            `json:"custom_parser"`
	Approvals    bool            `json:"approvals"`
	Name         string          `json:"name"`
	Fallbacks    []string        `json:"fallbacks"`
	Schema       json.RawMessage `json:"schema"`
	Streaming    bool            `json:"streaming"`
	MaxTurns     int             `json:"max_turns"`
	Input        []item          `json:"input"`
	Responses    []response      `json:"responses"`
}

func convert(items []item) []sdk.RunItem {
	out := []sdk.RunItem{}
	for _, v := range items {
		switch v.Type {
		case "message":
			out = append(out, sdk.RunItem{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: v.Text}})
		case "tool_call":
			out = append(out, sdk.RunItem{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: v.ID, Name: v.Name, Input: v.Arguments}})
		default:
			panic("unsupported script item: " + v.Type)
		}
	}
	return out
}
func normalize(items []sdk.RunItem) []item {
	out := []item{}
	for _, v := range items {
		switch v.Type {
		case sdk.RunItemMessage:
			out = append(out, item{Type: "message", Text: v.Message.Text})
		case sdk.RunItemToolCall:
			out = append(out, item{Type: "tool_call", ID: v.ToolCall.ID, Name: v.ToolCall.Name, Arguments: v.ToolCall.Input})
		case sdk.RunItemToolApproval:
			continue
		case sdk.RunItemToolOutput:
			out = append(out, item{Type: "tool_result", ID: v.ToolOutput.CallID, Content: v.ToolOutput.Content, IsError: v.ToolOutput.IsError})
		default:
			panic(fmt.Sprintf("unsupported runner item: %v", v.Type))
		}
	}
	return out
}

type model struct {
	script   []response
	requests []object
}

func (m *model) GetResponse(context.Context, sdk.ModelRequest) (*sdk.ModelResponse, error) {
	panic("Go runner unexpectedly used GetResponse")
}
func (m *model) StreamResponse(_ context.Context, req sdk.ModelRequest) (*sdk.ModelStream, error) {
	names := []string{}
	for _, t := range req.Tools {
		names = append(names, t.Name())
	}
	index := len(m.requests)
	m.requests = append(m.requests, object{"model": req.Model, "instructions": req.Instructions, "input": normalize(req.Input), "tools": names})
	if index >= len(m.script) {
		return nil, errors.New("script exhausted")
	}
	s := m.script[index]
	if s.Error != "" {
		return nil, errors.New(s.Error)
	}
	r := &sdk.ModelResponse{Items: convert(s.Items), EndTurn: s.EndTurn, Usage: sdk.Usage{InputTokens: s.InputTokens, OutputTokens: s.OutputTokens}}
	events := make(chan sdk.ModelStreamEvent, len(s.Deltas)+1)
	for _, delta := range s.Deltas {
		events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamDelta, Delta: delta}
	}
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: r}
	close(events)
	done := make(chan *sdk.ModelResponse, 1)
	done <- r
	close(done)
	return sdk.NewModelStream(events, done), nil
}
func (*model) GetRetryAdvice(error) *sdk.ModelRetryAdvice {
	return &sdk.ModelRetryAdvice{ShouldRetry: true, Reason: "overloaded"}
}
func (m *model) GetModel(string) (sdk.Model, error) { return m, nil }
func (*model) Close() error                         { return nil }
func (*model) CalculateCost(sdk.Usage) float64      { return 0 }
func (*model) Provider() string                     { return "replay" }

type recordingHooks struct {
	sdk.NoOpRunHooks
	mu     sync.Mutex
	events []object
}

func (h *recordingHooks) append(event object) {
	h.mu.Lock()
	defer h.mu.Unlock()
	h.events = append(h.events, event)
}
func (h *recordingHooks) OnAgentStart(_ *sdk.RunContext, a *sdk.Agent) {
	h.append(object{"type": "agent_start", "agent": a.Name})
}
func (h *recordingHooks) OnLLMStart(_ *sdk.RunContext, a *sdk.Agent) {
	h.append(object{"type": "model_start", "agent": a.Name})
}
func (h *recordingHooks) OnLLMEnd(_ *sdk.RunContext, _ *sdk.Agent, r *sdk.ModelResponse) {
	h.append(object{"type": "model_end", "items": normalize(r.Items), "input_tokens": r.Usage.InputTokens, "output_tokens": r.Usage.OutputTokens})
}
func (h *recordingHooks) OnToolStart(_ *sdk.RunContext, _ *sdk.Agent, _ sdk.Tool, c sdk.ToolCallData) {
	h.append(object{"type": "tool_start", "id": c.ID, "name": c.Name, "arguments": c.Input})
}
func (h *recordingHooks) OnToolEnd(_ *sdk.RunContext, _ *sdk.Agent, _ sdk.Tool, c sdk.ToolCallData, r sdk.ToolResult) {
	h.append(object{"type": "tool_end", "id": c.ID, "output": r.Content, "is_error": r.IsError})
}
func (h *recordingHooks) OnAgentEnd(_ *sdk.RunContext, a *sdk.Agent, output any) {
	h.append(object{"type": "agent_end", "agent": a.Name, "output": output})
}

type approvalGate struct{ deny bool }

func (g approvalGate) ApproveTool(context.Context, sdk.ToolApprovalRequest) (bool, string, error) {
	return !g.deny, "", nil
}

func execute(s scenario) object {
	m := &model{script: s.Responses, requests: []object{}}
	dispatch := []object{}
	tool := &sdk.FunctionTool{ToolName: "echo", ToolDescription: "Echo text", Schema: json.RawMessage(`{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}`), ReadOnly: true,
		Fn: func(_ context.Context, input json.RawMessage) (string, error) {
			var args struct {
				Text string `json:"text"`
			}
			if err := json.Unmarshal(input, &args); err != nil {
				panic(err)
			}
			dispatch = append(dispatch, object{"name": "echo", "arguments": input, "output": "echo: " + args.Text})
			return "echo: " + args.Text, nil
		}}
	agent := &sdk.Agent{Name: "replay-agent", Model: "replay-model", Instructions: "Follow the replay script.", Tools: []sdk.Tool{tool}}
	if s.Approvals {
		approval := *tool
		approval.ToolName = "approval"
		approval.Approval = true
		approval.Fn = func(_ context.Context, input json.RawMessage) (string, error) {
			if !s.Resume || s.Deny {
				panic("unapproved effect executed")
			}
			var args struct {
				Text string `json:"text"`
			}
			if err := json.Unmarshal(input, &args); err != nil {
				panic(err)
			}
			output := "echo: " + args.Text
			dispatch = append(dispatch, object{"name": "approval", "arguments": input, "output": output})
			return output, nil
		}
		agent.Tools = append(agent.Tools, &approval)
	}
	agent.FallbackModels = s.Fallbacks
	if len(s.Schema) > 0 {
		var schema any
		if err := json.Unmarshal(s.Schema, &schema); err != nil {
			panic(err)
		}
		canonical, err := json.Marshal(schema)
		if err != nil {
			panic(err)
		}
		agent.OutputType = sdk.NewOutputSchema("final_output", canonical)
		if s.SchemaName != "" {
			agent.OutputType.Name = s.SchemaName
		}
		if s.SchemaStrict != nil {
			agent.OutputType.Strict = *s.SchemaStrict
		}
		if s.CustomParser {
			agent.OutputType.ParseFn = func(raw string) (any, error) {
				var value struct {
					N int `json:"n"`
				}
				if err := json.Unmarshal([]byte(raw), &value); err != nil {
					return nil, err
				}
				if value.N < 0 {
					return nil, errors.New("negative n")
				}
				return object{"accepted": value.N}, nil
			}
		}
	}
	trusted := s.Untrusted
	hooks := &recordingHooks{events: []object{}}
	cfg := sdk.RunConfig{Hooks: hooks, MaxToolOutputBytes: s.OutputCap, MaxTurns: s.MaxTurns, TracingDisabled: true, UntrustedToolOutputs: &trusted, ToolAccessLevel: sdk.ToolAccessLevelReadOnly, ModelCallTimeout: -1}
	runner := sdk.NewRunnerWithProvider(m)
	var result *sdk.RunResult
	var err error
	events := []object{}
	if s.Streaming {
		stream := runner.RunStreamed(context.Background(), agent, convert(s.Input), cfg)
		for ev := range stream.Events {
			switch ev.Type {
			case sdk.StreamEventRawResponse:
				if ev.Name != "model.delta" {
					panic("unexpected raw event: " + ev.Name)
				}
				events = append(events, object{"type": "text_delta", "delta": ev.Delta})
			case sdk.StreamEventRunItem:
				for _, v := range normalize([]sdk.RunItem{*ev.Item}) {
					events = append(events, object{"type": "item", "item": v})
				}
			default:
				panic(fmt.Sprintf("unexpected stream event: %v", ev.Type))
			}
		}
		result, err = stream.FinalResult(), stream.Err()
	} else if s.ChatLoop {
		if len(s.Input) != 0 {
			panic("chat loop fixture must start with empty history")
		}
		opts := sdk.ChatLoopOptions{Runner: runner, Agent: agent, RunConfig: cfg}
		if s.Resume {
			opts.ApprovalGate = approvalGate{deny: s.Deny}
		}
		result, err = sdk.NewChatLoop(opts).Run(context.Background())
	} else {
		result, err = runner.Run(context.Background(), agent, convert(s.Input), cfg)
	}
	status := "completed"
	var category any
	if err != nil {
		var max *sdk.MaxTurnsExceeded
		if !errors.As(err, &max) {
			panic(err)
		}
		if s.ChatLoop && result == nil {
			result = max.PartialResult
		}
		if result == nil || max.PartialResult != result {
			panic("missing partial result")
		}
		status, category = "incomplete", "max_turns"
	}
	if result.IsInterrupted() {
		status = "paused"
	}
	observation := object{"hooks": hooks.events, "requests": m.requests, "dispatch": dispatch, "events": events, "outcome": object{
		"status": status, "error": category, "final_output": result.FinalOutput, "history": normalize(result.FinalHistory), "new_items": normalize(result.NewItems),
		"response_count": len(result.RawResponses), "last_agent": result.LastAgent.Name, "input_tokens": result.Usage.InputTokens, "output_tokens": result.Usage.OutputTokens}}
	if s.Approvals {
		pending := []item{}
		for _, interruption := range result.AllInterruptions() {
			approval := interruption
			pending = append(pending, item{Type: "tool_call", ID: approval.ToolCallID, Name: approval.ToolName, Arguments: approval.ToolInput})
		}
		observation["pending"] = pending
	}
	return observation
}
func main() {
	var cases []scenario
	if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil {
		panic(err)
	}
	outputs := map[string]object{}
	for _, s := range cases {
		outputs[s.Name] = execute(s)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(outputs); err != nil {
		panic(err)
	}
}
