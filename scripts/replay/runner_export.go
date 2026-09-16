// GPL-3.0-only. Executes the pinned SDK runner; see runner_README.md.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"

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
	Name      string          `json:"name"`
	Fallbacks []string        `json:"fallbacks"`
	Schema    json.RawMessage `json:"schema"`
	Streaming bool            `json:"streaming"`
	MaxTurns  int             `json:"max_turns"`
	Input     []item          `json:"input"`
	Responses []response      `json:"responses"`
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
	}
	trusted := false
	cfg := sdk.RunConfig{MaxTurns: s.MaxTurns, TracingDisabled: true, UntrustedToolOutputs: &trusted, ToolAccessLevel: sdk.ToolAccessLevelReadOnly, ModelCallTimeout: -1}
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
				events = append(events, object{"type": "item", "item": normalize([]sdk.RunItem{*ev.Item})[0]})
			default:
				panic(fmt.Sprintf("unexpected stream event: %v", ev.Type))
			}
		}
		result, err = stream.FinalResult(), stream.Err()
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
		if result == nil || max.PartialResult != result {
			panic("missing partial result")
		}
		status, category = "incomplete", "max_turns"
	}
	return object{"requests": m.requests, "dispatch": dispatch, "events": events, "outcome": object{
		"status": status, "error": category, "final_output": result.FinalOutput, "history": normalize(result.FinalHistory), "new_items": normalize(result.NewItems),
		"response_count": len(result.RawResponses), "last_agent": result.LastAgent.Name, "input_tokens": result.Usage.InputTokens, "output_tokens": result.Usage.OutputTokens}}
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
