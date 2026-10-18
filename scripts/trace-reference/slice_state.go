package main

import agent "github.com/gratefulagents/sdk/pkg/agentsdk"

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
