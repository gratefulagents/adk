package main

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type conversationCase struct {
	Name   string          `json:"name"`
	Input  json.RawMessage `json:"input"`
	Output any             `json:"output"`
}

func conversationObservation[I, O any](name string, input I, run func(I) O) conversationCase {
	b, err := json.Marshal(input)
	if err != nil {
		panic(err)
	}
	// Preserve the pre-call input independently of SDK mutation and sibling cases.
	var independent I
	if err := json.Unmarshal(b, &independent); err != nil {
		panic(err)
	}
	return conversationCase{Name: name, Input: b, Output: run(independent)}
}

type tailInput struct {
	Messages         []sdk.ConversationMessage `json:"messages"`
	State            sdk.WorkingState          `json:"state"`
	ExcludeMessageID int64                     `json:"exclude_message_id"`
	Limit            int                       `json:"limit"`
}
type itemsOutput struct {
	Items []sdk.LLMRunItemSnapshot `json:"items"`
	Count int                      `json:"count"`
}
type stateInput struct {
	State sdk.WorkingState `json:"state"`
}
type textOutput struct {
	Text string `json:"text"`
}
type goalInput struct {
	RawReply        string `json:"raw_reply"`
	EffectivePrompt string `json:"effective_prompt"`
}
type truncateInput struct {
	Text string `json:"text"`
	Max  int    `json:"max"`
}
type summaryInput struct {
	Items []sdk.LLMRunItemSnapshot `json:"items"`
}
type toolSummaryInput struct {
	Items []sdk.LLMRunItemSnapshot `json:"items"`
	Limit int                      `json:"limit"`
}
type toolSummaryOutput struct {
	Summaries []string `json:"summaries"`
}
type queueInput struct {
	Messages          []sdk.UserMessage `json:"messages"`
	ConsumedImmediate []int64           `json:"consumed_immediate"`
}
type selectionOutput struct {
	Message                sdk.UserMessage `json:"message"`
	OK                     bool            `json:"ok"`
	SkipCursor             int64           `json:"skip_cursor"`
	Immediate              bool            `json:"immediate"`
	ConsumedImmediateAfter []int64         `json:"consumed_immediate_after"`
}
type immediateOutput struct {
	Items                  []sdk.LLMRunItemSnapshot `json:"items"`
	Count                  int                      `json:"count"`
	Cursor                 int64                    `json:"cursor"`
	ConsumedImmediateAfter []int64                  `json:"consumed_immediate_after"`
}

func consumedSet(ids []int64) map[int64]struct{} {
	out := map[int64]struct{}{}
	for _, id := range ids {
		out[id] = struct{}{}
	}
	return out
}
func consumedIDs(set map[int64]struct{}) []int64 {
	out := make([]int64, 0, len(set))
	for id := range set {
		out = append(out, id)
	}
	sort.Slice(out, func(i, j int) bool { return out[i] < out[j] })
	return out
}
func selectObservation(in queueInput) selectionOutput {
	consumed := consumedSet(in.ConsumedImmediate)
	msg, ok, cursor, immediate := sdk.SelectNextUserMessage(in.Messages, consumed)
	return selectionOutput{msg, ok, cursor, immediate, consumedIDs(consumed)}
}
func collectObservation(in queueInput) immediateOutput {
	consumed := consumedSet(in.ConsumedImmediate)
	items, cursor := sdk.CollectImmediateRunItems(in.Messages, consumed)
	return immediateOutput{sdk.SnapshotRunItems(items), len(items), cursor, consumedIDs(consumed)}
}

type resultHelperInput struct {
	FinalOutput   json.RawMessage          `json:"final_output,omitempty"`
	NewItems      []sdk.LLMRunItemSnapshot `json:"new_items"`
	FinalHistory  []sdk.LLMRunItemSnapshot `json:"final_history"`
	Interruption  *sdk.Interruption        `json:"interruption"`
	Interruptions []*sdk.Interruption      `json:"interruptions"`
	LegacyShape   string                   `json:"legacy_shape"`
	SourceOnly    bool                     `json:"source_only"`
}
type resultHelperOutput struct {
	FinalText        string                   `json:"final_text"`
	InputList        []sdk.LLMRunItemSnapshot `json:"input_list"`
	InputCount       int                      `json:"input_count"`
	IsInterrupted    bool                     `json:"is_interrupted"`
	AllInterruptions []*sdk.Interruption      `json:"all_interruptions"`
}

func resultObservation(in resultHelperInput) resultHelperOutput {
	result := &sdk.RunResult{NewItems: restore(in.NewItems), FinalHistory: restore(in.FinalHistory), Interruption: in.Interruption, Interruptions: in.Interruptions}
	if len(in.FinalOutput) > 0 {
		if err := json.Unmarshal(in.FinalOutput, &result.FinalOutput); err != nil {
			panic(err)
		}
	}
	items := result.ToInputList()
	return resultHelperOutput{result.FinalText(), sdk.SnapshotRunItems(items), len(items), result.IsInterrupted(), result.AllInterruptions()}
}

func conversationFixture() object {
	image := []sdk.ImageAttachment{{MediaType: "image/png", Data: "aGVsbG8=", Detail: "high"}, {MediaType: "image/jpeg", Data: "AAEC", Detail: "low"}}
	tails := []conversationCase{}
	addTail := func(name string, in tailInput) {
		tails = append(tails, conversationObservation(name, in, func(in tailInput) itemsOutput {
			items := sdk.BuildConversationTail(in.Messages, in.State, in.ExcludeMessageID, in.Limit)
			return itemsOutput{sdk.SnapshotRunItems(items), len(items)}
		}))
	}
	addTail("empty", tailInput{})
	messages := []sdk.ConversationMessage{
		{ID: 1, Role: "user", Content: "below floor"},
		{ID: 2, Role: "assistant", Content: "at floor"},
		{ID: 3, Role: "user", Content: "excluded current"},
		{ID: 4, Role: "user", Content: " \n\t "},
		{ID: 5, Role: "assistant", Content: "  assistant\nsummary 界🙂  "},
		{ID: 6, Role: "system", Content: "  system\r\nsummary  "},
		{ID: 7, Role: "user", Content: " \n ", Images: image},
		{ID: 8, Role: "tool", Content: "unknown role stays user"},
		{ID: 9, Role: "Assistant", Content: "case-sensitive role"},
	}
	addTail("floor-exclude-roles-images", tailInput{messages, sdk.WorkingState{HistoryFloorMessageID: 2}, 3, 8})
	addTail("last-two-after-filter", tailInput{messages, sdk.WorkingState{HistoryFloorMessageID: 2}, 3, 2})
	addTail("all-filtered", tailInput{messages, sdk.WorkingState{HistoryFloorMessageID: 9}, 0, 8})
	many := []sdk.ConversationMessage{}
	for i := int64(1); i <= 11; i++ {
		many = append(many, sdk.ConversationMessage{ID: i, Role: "user", Content: fmt.Sprintf("message-%d", i)})
	}
	for _, limit := range []int{0, -2, 1, 20} {
		addTail(fmt.Sprintf("limit-%d", limit), tailInput{Messages: many, Limit: limit})
	}
	addTail("slice-order-not-id-sort", tailInput{Messages: []sdk.ConversationMessage{{ID: 90, Content: "first"}, {ID: 7, Content: "second"}, {ID: 60, Content: "last"}}, Limit: 2})
	addTail("nonpositive-exclude-disabled", tailInput{Messages: []sdk.ConversationMessage{{ID: -1, Content: "negative"}, {ID: 0, Content: "zero"}, {ID: 1, Content: "positive"}}, State: sdk.WorkingState{HistoryFloorMessageID: -2}, ExcludeMessageID: -1})
	addTail("unicode-1200-rune-cap", tailInput{Messages: []sdk.ConversationMessage{{ID: 1, Role: "assistant", Content: strings.Repeat("界🙂", 601), Images: image}}})

	states := []conversationCase{}
	for _, tc := range []struct {
		name  string
		state sdk.WorkingState
	}{
		{"empty", sdk.WorkingState{}},
		{"all-fields-last-four", sdk.WorkingState{Goal: "  finish\nshipping  ", CurrentMode: " plan\nraw ", CurrentStep: " inspect\ncode ", LastUserMessage: "new direction", LastAssistantSummary: "checked\nsource", RecentTurnSummaries: []string{"discard-me", "one", "two\nlines", "", "five"}, HistoryFloorMessageID: 99, LastResponseID: "not-in-context", Data: map[string]any{"ignored": true}}},
		{"same-goal-direction-omitted", sdk.WorkingState{Goal: "same", LastUserMessage: "same"}},
		{"raw-equality-before-trimming", sdk.WorkingState{Goal: " same ", LastUserMessage: "same"}},
		{"whitespace-fields-not-empty", sdk.WorkingState{Goal: " \n ", CurrentStep: "\t", LastAssistantSummary: " ", RecentTurnSummaries: []string{" "}}},
		{"unicode-320-rune-caps", sdk.WorkingState{Goal: strings.Repeat("界", 321), CurrentStep: strings.Repeat("🙂", 321), LastUserMessage: strings.Repeat("é", 321), LastAssistantSummary: strings.Repeat("λ", 321), RecentTurnSummaries: []string{strings.Repeat("文", 321)}}},
		{"non-context-fields-only", sdk.WorkingState{HistoryFloorMessageID: 22, LastResponseID: "response", Data: map[string]any{"goal": "not-used"}}},
	} {
		states = append(states, conversationObservation(tc.name, stateInput{tc.state}, func(in stateInput) textOutput { return textOutput{sdk.BuildWorkingStateContext(in.State)} }))
	}
	goals := []conversationCase{}
	for i, in := range []goalInput{
		{"", " effective\nprompt "}, {" \n\t", " "}, {" APPROVE ", " original "}, {"Deny", "original"}, {"request changes", "original"}, {"request_changes", "original"},
		{"APPROVE: ship", "original"}, {"deny: nope", "original"}, {"Request Changes: revise", "original"}, {"REQUEST_CHANGES: revise", "original"},
		{"approve", " "}, {"request changes: revise", ""}, {" approve later ", "original"}, {"approve : spaced colon", "original"}, {" new\n方向🙂 ", "original"},
	} {
		goals = append(goals, conversationObservation(fmt.Sprintf("reply-%02d", i), in, func(in goalInput) textOutput {
			return textOutput{sdk.DeriveWorkingStateGoal(in.RawReply, in.EffectivePrompt)}
		}))
	}
	truncations := []conversationCase{}
	for i, in := range []truncateInput{
		{"", 1}, {" \n\t ", 0}, {" a\n b\t c\r\nd  ", 0}, {" a\n b\t c\r\nd  ", -3},
		{"界🙂é", 1}, {"界🙂é", 2}, {"界🙂é", 3}, {"界🙂é", 4}, {"e\u0301🙂", 1}, {"a\nb", 2}, {"\u00a0 hi\u2003", 2},
	} {
		truncations = append(truncations, conversationObservation(fmt.Sprintf("text-%02d", i), in, func(in truncateInput) textOutput { return textOutput{sdk.TruncateContextText(in.Text, in.Max)} }))
	}

	assistant := func(text, agent string) sdk.LLMRunItemSnapshot {
		return sdk.LLMRunItemSnapshot{Type: "message", AgentName: agent, MessageText: text}
	}
	output := func(text string, isError bool) sdk.LLMRunItemSnapshot {
		return sdk.LLMRunItemSnapshot{Type: "tool_output", ToolOutput: &sdk.ToolOutputData{CallID: "call", Content: text, IsError: isError}}
	}
	calls := []sdk.LLMRunItemSnapshot{call("1", "zeta"), call("2", "beta"), call("3", "alpha"), call("4", "beta"), call("5", "alpha"), call("6", "delta"), call("7", "gamma"), call("8", ""), message("not a tool")}
	summaries := []conversationCase{}
	for _, tc := range []struct {
		name  string
		items []sdk.LLMRunItemSnapshot
	}{
		{"empty", nil},
		{"user-and-image-not-assistant", []sdk.LLMRunItemSnapshot{message("user must not count"), {Type: "message", AgentName: "assistant", MessageImages: image}, {Type: "reasoning", Reasoning: &sdk.LLMReasoning{Text: "not a message"}}}},
		{"unique-assistant-first-two-tools-issues", append([]sdk.LLMRunItemSnapshot{message("ignore user"), assistant(" first\nanswer ", "worker"), assistant("first answer", "other-worker"), assistant("second", "system-summary"), assistant("third omitted", "worker"), output("success omitted", false), output(" failure\none ", true), output("failure one", true), output("second failure", true), output("third failure omitted", true)}, calls...)},
		{"unique-successes-without-assistant", []sdk.LLMRunItemSnapshot{output(" result\none ", false), output("result one", false), output("two", false), output("three omitted", false), output("bad", true)}},
		{"dedup-after-220-rune-truncation", []sdk.LLMRunItemSnapshot{assistant(strings.Repeat("界", 220)+"a", "worker"), assistant(strings.Repeat("界", 220)+"b", "worker"), assistant("second unique", "worker")}},
		{"results-and-issues-120-rune-cap", []sdk.LLMRunItemSnapshot{output(strings.Repeat("🙂", 121), false), output(strings.Repeat("é", 121), true)}},
	} {
		summaries = append(summaries, conversationObservation(tc.name, summaryInput{tc.items}, func(in summaryInput) textOutput { return textOutput{sdk.BuildAssistantTurnSummary(restore(in.Items))} }))
	}
	toolSummaries := []conversationCase{}
	for _, limit := range []int{0, -1, 1, 4, 20} {
		toolSummaries = append(toolSummaries, conversationObservation(fmt.Sprintf("frequency-ties-limit-%d", limit), toolSummaryInput{calls, limit}, func(in toolSummaryInput) toolSummaryOutput {
			return toolSummaryOutput{sdk.SummarizeTurnToolCalls(restore(in.Items), in.Limit)}
		}))
	}
	toolSummaries = append(toolSummaries, conversationObservation("no-calls", toolSummaryInput{[]sdk.LLMRunItemSnapshot{message("hello")}, 0}, func(in toolSummaryInput) toolSummaryOutput {
		return toolSummaryOutput{sdk.SummarizeTurnToolCalls(restore(in.Items), in.Limit)}
	}))

	selections, immediates := []conversationCase{}, []conversationCase{}
	for _, tc := range []struct {
		name  string
		input queueInput
	}{
		{"empty", queueInput{}},
		{"all-blank", queueInput{Messages: []sdk.UserMessage{{ID: 1, Content: " "}, {ID: 2, Content: "\n", Mode: "immediate"}}}},
		{"queue-before-immediates", queueInput{Messages: []sdk.UserMessage{{ID: 20, Content: "queued", Mode: "enqueue"}, {ID: 21, Content: " first\nimmediate ", Mode: "immediate"}, {ID: 22, Content: "second", Mode: "immediate"}}}},
		{"consumed-prefix-queue-barrier", queueInput{Messages: []sdk.UserMessage{{ID: 1, Content: " "}, {ID: 2, Content: "consumed", Mode: "immediate"}, {ID: 3, Content: "queue", Mode: "enqueue"}, {ID: 4, Content: "consumed later", Mode: "immediate"}, {ID: 5, Content: "fresh", Mode: "immediate"}}, ConsumedImmediate: []int64{2, 4, 99}}},
		{"all-consumed-with-trailing-blank", queueInput{Messages: []sdk.UserMessage{{ID: 1, Content: "done", Mode: "immediate"}, {ID: 2, Content: " "}}, ConsumedImmediate: []int64{1}}},
		{"immediate-prefix-then-queue", queueInput{Messages: []sdk.UserMessage{{ID: 10, Content: "now", Mode: "immediate"}, {ID: 11, Content: " "}, {ID: 12, Content: "later", Mode: "enqueue"}, {ID: 13, Content: "next now", Mode: "immediate"}}}},
		{"image-only-immediate", queueInput{Messages: []sdk.UserMessage{{ID: 1, Content: " \n ", Mode: "immediate", Images: image}}}},
		{"image-only-queue-barrier", queueInput{Messages: []sdk.UserMessage{{ID: 1, Images: image, Mode: "enqueue"}, {ID: 2, Content: "now", Mode: "immediate"}}}},
		{"unknown-mode-is-queued", queueInput{Messages: []sdk.UserMessage{{ID: 1, Content: " default\nqueue "}, {ID: 2, Content: "case-sensitive", Mode: "Immediate"}}, ConsumedImmediate: []int64{1, 2}}},
		{"out-of-order-prefix-not-max-id", queueInput{Messages: []sdk.UserMessage{{ID: 90, Content: " "}, {ID: 7, Content: "already consumed", Mode: "immediate"}, {ID: 60, Content: "queued", Mode: "enqueue"}, {ID: 3, Content: "now", Mode: "immediate"}}, ConsumedImmediate: []int64{7}}},
		{"out-of-order-immediate-slice-order", queueInput{Messages: []sdk.UserMessage{{ID: 90, Content: "first", Mode: "immediate"}, {ID: 7, Content: "second", Mode: "immediate"}, {ID: 60, Content: "third", Mode: "immediate"}}}},
		{"duplicate-immediate-id", queueInput{Messages: []sdk.UserMessage{{ID: 5, Content: "first", Mode: "immediate"}, {ID: 5, Content: "second same id", Mode: "immediate"}}}},
	} {
		selections = append(selections, conversationObservation(tc.name, tc.input, selectObservation))
		immediates = append(immediates, conversationObservation(tc.name, tc.input, collectObservation))
	}

	results := []conversationCase{}
	for _, tc := range []struct{ name, value string }{{"missing", ""}, {"null", "null"}, {"string", `"  final\n界🙂  "`}, {"number", "42"}, {"object", `{"answer":"not text"}`}, {"array", `["not text"]`}, {"boolean", "true"}} {
		results = append(results, conversationObservation("final-output-"+tc.name, resultHelperInput{FinalOutput: json.RawMessage(tc.value), LegacyShape: "empty"}, resultObservation))
	}
	first := &sdk.Interruption{ToolName: "write", ToolCallID: "c1", ToolInput: json.RawMessage(`{"path":"one"}`), ParentContext: []sdk.LLMRunItemSnapshot{assistant("prior context", "parent")}}
	second := &sdk.Interruption{ToolName: "delete", ToolCallID: "c2", ToolInput: json.RawMessage(`{"path":"two"}`)}
	for _, tc := range []struct {
		name       string
		singular   *sdk.Interruption
		plural     []*sdk.Interruption
		shape      string
		sourceOnly bool
	}{
		{"empty-plural", nil, []*sdk.Interruption{}, "empty", false},
		{"singular-legacy", first, nil, "singular-only", false},
		{"singular-empty-plural", first, []*sdk.Interruption{}, "singular-only", false},
		{"consistent-plural", first, []*sdk.Interruption{first, second}, "consistent", false},
		{"plural-without-singular", nil, []*sdk.Interruption{first, second}, "contradictory-plural-only", true},
		{"singular-disagrees-with-plural", first, []*sdk.Interruption{second}, "contradictory-singular-not-first", true},
	} {
		results = append(results, conversationObservation(tc.name, resultHelperInput{NewItems: []sdk.LLMRunItemSnapshot{assistant("new", "worker"), {Type: "message", MessageImages: image}}, FinalHistory: []sdk.LLMRunItemSnapshot{message("old history must not be returned"), assistant("new", "worker"), {Type: "message", MessageImages: image}}, Interruption: tc.singular, Interruptions: tc.plural, LegacyShape: tc.shape, SourceOnly: tc.sourceOnly}, resultObservation))
	}
	return object{"sdk_revision": pin, "schema_version": 1, "build_conversation_tail": tails, "build_working_state_context": states, "derive_working_state_goal": goals, "truncate_context_text": truncations, "build_assistant_turn_summary": summaries, "summarize_turn_tool_calls": toolSummaries, "select_next_user_message": selections, "collect_immediate_run_items": immediates, "result_helpers": results}
}
