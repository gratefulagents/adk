package main

import (
	"encoding/json"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func scenarios() []scenario {
	alpha := &agentsdk.Agent{Name: "alpha"}
	beta := &agentsdk.Agent{Name: "beta"}
	user := agentsdk.RunItem{Type: agentsdk.RunItemMessage, Message: &agentsdk.MessageOutput{Text: " user <input>\n"}}
	assistant := agentsdk.RunItem{Type: agentsdk.RunItemMessage, Agent: alpha, Message: &agentsdk.MessageOutput{Text: "assistant α"}}
	commentary := agentsdk.RunItem{Type: agentsdk.RunItemMessage, Agent: alpha, Message: &agentsdk.MessageOutput{Text: "checking", Phase: "commentary"}}
	final := agentsdk.RunItem{Type: agentsdk.RunItemMessage, Agent: beta, Message: &agentsdk.MessageOutput{Text: "done", Phase: "final_answer"}}
	compaction := agentsdk.RunItem{Type: agentsdk.RunItemCompaction, Agent: beta, Compaction: &agentsdk.CompactionData{ID: "cmp-1", Content: "summary\n", EncryptedContent: "opaque==", CreatedBy: "provider"}}
	stripped := []agentsdk.RunItem{
		{Type: agentsdk.RunItemToolCall, Agent: alpha, ToolCall: &agentsdk.ToolCallData{ID: "call-1", Name: "lookup", Input: json.RawMessage(`{"query":"x","n":2}`)}},
		{Type: agentsdk.RunItemToolOutput, Agent: alpha, ToolOutput: &agentsdk.ToolOutputData{CallID: "call-1", Content: "failure", IsError: true}},
		{Type: agentsdk.RunItemHandoffCall, Agent: alpha, HandoffCall: &agentsdk.HandoffCallData{FromAgent: "alpha", ToAgent: "beta"}},
		{Type: agentsdk.RunItemHandoffOutput, Agent: beta, HandoffOutput: &agentsdk.HandoffOutputData{FromAgent: "alpha", ToAgent: "beta"}},
		{Type: agentsdk.RunItemReasoning, Agent: beta, Reasoning: &agentsdk.ReasoningData{ID: "reason-1", Text: "private", Signature: "sig", RedactedData: "redacted", EncryptedContent: "encrypted"}},
		{Type: agentsdk.RunItemToolApproval, Agent: beta, ToolApproval: &agentsdk.ToolApprovalData{ToolName: "lookup", Input: json.RawMessage(`{"query":"x"}`), CallID: "call-1", Approved: true}},
	}
	cases := []scenario{
		{Name: "nil_input"},
		{Name: "empty_input", Input: []agentsdk.RunItem{}, NewItems: []agentsdk.RunItem{}},
		{Name: "user_message", Input: []agentsdk.RunItem{user}},
		{Name: "assistant_message", Input: []agentsdk.RunItem{assistant}},
		{Name: "phased_messages", Input: []agentsdk.RunItem{commentary, final}},
		{Name: "compaction", Input: []agentsdk.RunItem{compaction}},
		{Name: "distinct_agent_history", Input: []agentsdk.RunItem{user, assistant, final, assistant}},
		{Name: "agent_named_system", SourceOnly: []string{"agent-label-not-system-role"}, Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Agent: &agentsdk.Agent{Name: "system"}, Message: &agentsdk.MessageOutput{Text: "agent label only"}}}},
		{Name: "agent_named_developer", SourceOnly: []string{"agent-label-not-developer-role"}, Input: []agentsdk.RunItem{{Type: agentsdk.RunItemMessage, Agent: &agentsdk.Agent{Name: "developer"}, Message: &agentsdk.MessageOutput{Text: "agent label only"}}}},
		{Name: "zero_value_item", SourceOnly: []string{"message-without-payload"}, Input: []agentsdk.RunItem{{}}},
		{Name: "unknown_types", SourceOnly: []string{"unknown-native-item-types"}, Input: []agentsdk.RunItem{{Type: agentsdk.RunItemType(-1)}, {Type: agentsdk.RunItemType(99), Agent: beta}}},
	}
	names := []string{"tool_call", "tool_output", "handoff_call", "handoff_output", "reasoning", "tool_approval"}
	for i, item := range stripped {
		s := scenario{Name: "strip_" + names[i], Input: []agentsdk.RunItem{item}}
		if i == 3 || i == 5 {
			s.SourceOnly = []string{"no-native-" + names[i] + "-run-item"}
		}
		cases = append(cases, s)
	}
	allReasons := []string{"no-native-handoff_output-run-item", "no-native-tool_approval-run-item"}
	return append(cases,
		scenario{Name: "all_six_stripped", SourceOnly: allReasons, Input: stripped},
		scenario{Name: "mixed_sequence", SourceOnly: allReasons, Input: []agentsdk.RunItem{stripped[0], user, stripped[1], assistant, stripped[2], commentary, stripped[3], compaction, stripped[4], final, stripped[5], user}},
		scenario{Name: "native_mixed_sequence", Input: []agentsdk.RunItem{stripped[0], user, stripped[1], assistant, stripped[2], commentary, compaction, stripped[4], final, user}},
		scenario{Name: "nil_input_nonempty_new_items", Input: nil, NewItems: []agentsdk.RunItem{assistant, stripped[0], compaction}},
		scenario{Name: "empty_input_nonempty_new_items", Input: []agentsdk.RunItem{}, NewItems: []agentsdk.RunItem{user, final}},
		scenario{Name: "new_items_ignored", Input: []agentsdk.RunItem{user, stripped[0], assistant}, NewItems: []agentsdk.RunItem{final, stripped[1], compaction}},
	)
}
