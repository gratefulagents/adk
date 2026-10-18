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

func imagePayloadCases() []map[string]any {
	inputs := []string{
		`null`, `{}`, `{"images":null}`, `{"images":[]}`, `{"images":[null]}`,
		`{"images":[{"data":"one"},{"data":"two"}],"images":[{"data":"short"}],"images":[null,null]}`,
		`{"images":[{"data":"one"}],"images":[],"images":[null]}`,
		`{"images":[{"data":"one"}],"images":null,"images":[null]}`,
	}
	var cases []map[string]any
	for _, input := range inputs {
		var message agent.MessageOutput
		var output agent.ToolOutputData
		must(json.Unmarshal([]byte(input), &message))
		must(json.Unmarshal([]byte(input), &output))
		snapshots := agent.SnapshotRunItems([]agent.RunItem{
			{Type: agent.RunItemMessage, Message: &message},
			{Type: agent.RunItemToolOutput, ToolOutput: &output},
		})
		messageBytes, err := json.Marshal(message)
		must(err)
		outputBytes, err := json.Marshal(output)
		must(err)
		requestInput := `{"input_items":[{"tool_output":` + input + `}]}`
		var request agent.LLMRequestSnapshot
		must(json.Unmarshal([]byte(requestInput), &request))
		requestBytes, err := json.Marshal(request)
		must(err)
		cases = append(cases, map[string]any{
			"input": input, "images_nil": message.Images == nil,
			"output_images_nil":           output.Images == nil,
			"message_snapshot_images_nil": snapshots[0].MessageImages == nil,
			"output_snapshot_images_nil":  snapshots[1].ToolOutput.Images == nil,
			"message_encoded":             string(messageBytes), "output_encoded": string(outputBytes),
			"request_input": requestInput, "request_encoded": string(requestBytes),
			"output_present": request.InputItems[0].ToolOutput != nil,
		})
	}
	return cases
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
