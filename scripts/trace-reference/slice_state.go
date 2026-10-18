package main

import (
	"encoding/json"
	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
)

func imageSliceStates(items []agent.LLMRunItemSnapshot) []bool {
	out := []bool{}
	for _, item := range items {
		out = append(out, item.MessageImages == nil)
	}
	return out
}

func requestSliceState(snapshot agent.LLMRequestSnapshot) map[string]any {
	return map[string]any{
		"input_items":    snapshot.InputItems == nil,
		"tools":          snapshot.Tools == nil,
		"stop_sequences": snapshot.Settings.StopSequences == nil,
		"message_images": imageSliceStates(snapshot.InputItems),
	}
}

func responseSliceState(snapshot agent.LLMResponseSnapshot) map[string]any {
	return map[string]any{
		"items":           snapshot.Items == nil,
		"texts":           snapshot.Texts == nil,
		"reasoning":       snapshot.Reasoning == nil,
		"reasoning_texts": snapshot.ReasoningTexts == nil,
		"thinking_texts":  snapshot.ThinkingTexts == nil,
		"tool_calls":      snapshot.ToolCalls == nil,
		"message_images":  imageSliceStates(snapshot.Items),
	}
}

func requestInPlaceCases() []map[string]any {
	inputs := [][2]string{
		{`{"settings":{"stop_sequences":[]}}`, `{}`},
		{`{"settings":{"stop_sequences":[]}}`, `{"settings":{}}`},
		{`{"settings":{"stop_sequences":[]}}`, `{"settings":null}`},
		{`{"input_items":[],"tools":[],"settings":{"stop_sequences":[]}}`, `{}`},
		{`{"input_items":[{"message_images":[]}]}`, `{"input_items":[{}]}`},
		{`{"input_items":[{"message_images":[]}]}`, `{"input_items":[null]}`},
	}
	var cases []map[string]any
	for _, input := range inputs {
		var snapshot agent.LLMRequestSnapshot
		must(json.Unmarshal([]byte(input[0]), &snapshot))
		must(json.Unmarshal([]byte(input[1]), &snapshot))
		encoded, err := json.Marshal(snapshot)
		must(err)
		cases = append(cases, map[string]any{"initial": input[0], "input": input[1], "encoded": string(encoded), "nil_slices": requestSliceState(snapshot)})
	}
	return cases
}
