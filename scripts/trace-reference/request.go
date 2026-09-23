package main

import (
	"encoding/json"
	"strings"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
)

// Uses the pinned SDK builder, not copied snapshot or estimation logic.
func nativeRequestCases() map[string]string {
	a := &agent.Agent{Name: "A"}
	temperature, parallel := 0.5, false
	full := agent.ModelRequest{
		Model: "fixture/model", PromptCacheKey: "private-cache-key", Instructions: "policy <>&\u2028\u2029",
		Input: []agent.RunItem{
			{Type: agent.RunItemMessage, Message: &agent.MessageOutput{Text: "user"}},
			{Type: agent.RunItemMessage, Agent: a, Message: &agent.MessageOutput{Text: "answer <>&", Phase: "commentary"}},
			{Type: agent.RunItemToolCall, Agent: a, ToolCall: &agent.ToolCallData{ID: "call", Name: "tool", Input: json.RawMessage(`{"x":"<>&"}`)}},
			{Type: agent.RunItemToolOutput, Agent: a, ToolOutput: &agent.ToolOutputData{CallID: "call", Content: "done"}},
			{Type: agent.RunItemReasoning, Agent: a, Reasoning: &agent.ReasoningData{ID: "reason", Text: "reasoning", Signature: "signature"}},
		},
		Tools:        []agent.Tool{&agent.FunctionTool{ToolName: "tool", ToolDescription: "description", Schema: json.RawMessage(`{"properties":{"x":{"type":"string"}},"type":"object"}`), ReadOnly: true, Approval: true, Timeout: 30}},
		Settings:     agent.ModelSettings{Temperature: &temperature, MaxTokens: 100, ParallelToolCalls: &parallel, ThinkingBudget: 200, StopSequences: []string{"stop"}},
		OutputSchema: &agent.OutputSchema{Name: "answer", Schema: json.RawMessage(`{"type":"object"}`), Strict: true},
	}
	approved := full
	approved.Input = append([]agent.RunItem(nil), full.Input[:3]...)
	approved.Input = append(approved.Input, agent.RunItem{Type: agent.RunItemToolApproval, Agent: a, ToolApproval: &agent.ToolApprovalData{ToolName: "tool", Input: json.RawMessage(`{"x":"<>&"}`), CallID: "call", Approved: true}})
	approved.Input = append(approved.Input, full.Input[3:]...)
	cases := map[string]agent.ModelRequest{
		"empty":          {Model: "fixture/model"},
		"full":           full,
		"approved":       approved,
		"compaction_cap": {Input: []agent.RunItem{{Type: agent.RunItemCompaction, Agent: a, Compaction: &agent.CompactionData{EncryptedContent: strings.Repeat("x", 80004)}}}},
		"negative_max":   {Settings: agent.ModelSettings{MaxTokens: -1, ThinkingBudget: 20000}},
	}
	result := map[string]string{}
	for name, request := range cases {
		encoded, err := json.Marshal(agent.BuildLLMRequestSnapshot("B", request))
		must(err)
		result[name] = string(encoded)
	}
	return result
}
