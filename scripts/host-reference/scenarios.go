package main

import (
	"encoding/json"
	"fmt"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	mode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

func message(text string) sdk.LLMRunItemSnapshot {
	return sdk.LLMRunItemSnapshot{Type: "message", MessageText: text}
}
func call(id, name string) sdk.LLMRunItemSnapshot {
	return sdk.LLMRunItemSnapshot{Type: "tool_call", ToolCall: &sdk.LLMToolCall{ID: id, Name: name, Input: json.RawMessage(`{"value":"secret", "run_id":"user-run-123", "timestamp":"2024-01-02T03:04:05Z"}`)}}
}
func reply(items ...sdk.LLMRunItemSnapshot) response {
	return response{Items: items, Usage: sdk.Usage{Requests: 1, InputTokens: 7, OutputTokens: 3}}
}
func baseline(name string) scenario {
	return scenario{Name: name, Runs: 1, Permission: sdk.PermissionModeDangerFullAccess, Pages: []page{{Messages: []sdk.UserMessage{{ID: 1, Content: "  user run_id=user-run-123 timestamp=2024-01-02T03:04:05Z  "}}, Next: sdk.Cursor{MessageID: 1, Token: "page-1"}}}, Responses: []response{reply(message("done"))}, BaseTools: []toolSpec{{Name: "read", ReadOnly: true, Output: "read result"}}, FactoryTools: []toolSpec{{Name: "write", Output: "secret output"}}, MaxTurns: 3}
}
func approvals(name string) scenario {
	s := baseline(name)
	s.Permission = sdk.PermissionModeWorkspaceWrite
	s.Gate = true
	s.FactoryTools = append(s.FactoryTools, toolSpec{Name: "write_b", Output: "second effect"})
	s.Responses = []response{reply(call("c1", "write"), call("c2", "write_b")), reply(message("both done"))}
	s.Decisions = []decision{{Approved: true}, {Approved: true}}
	return s
}
func scenarios() []scenario {
	out := []scenario{}
	s := baseline("preparation-order-and-handoff")
	s.Directive = "  mode directive  "
	s.AdditionalInstructions = "host instructions"
	s.WorkingStateContext = "old state"
	s.Snapshot = &mode.TemplateSpec{Name: "read-mode", ToolAccess: "read-only", Instructions: "NOT consumed as directive", Constraints: &mode.Constraints{MaxTurns: 8, SubAgentMaxTurns: 4}}
	s.MaxTurns = 0
	s.State = sdk.WorkingState{Goal: "goal", CurrentMode: "plan", CurrentStep: "inspect", LastUserMessage: "direction", LastAssistantSummary: "already checked", RecentTurnSummaries: []string{"one", "two"}, Data: map[string]any{"run_id": "user-run-123"}}
	s.Handoff = []sdk.LLMRunItemSnapshot{{Type: "message", AgentName: "prior-agent", MessageText: "handoff assistant"}, message("handoff user")}
	s.Rules = []sdk.GuardrailRule{{Name: "input", Type: "tool-input", Regex: "never-match", ToolPattern: "write*"}, {Name: "output", Type: "tool-output", Regex: "never-match", Action: "block"}}
	out = append(out, s)
	s = baseline("explicit-config-precedence")
	s.Permission = sdk.PermissionModeReadOnly
	s.Access = sdk.ToolAccessLevelFull
	s.Policy = &sdk.ToolPolicy{ApprovalRequired: false}
	s.MaxTurns = 2
	s.SubAgentMaxTurns = 5
	s.Snapshot = &mode.TemplateSpec{ToolAccess: "full", Constraints: &mode.Constraints{MaxTurns: 9, SubAgentMaxTurns: 9}}
	s.State = sdk.WorkingState{Goal: "alone does not replace working state", LastUserMessage: "also not a trigger"}
	s.WorkingStateContext = "retained host state"
	s.Responses = []response{reply(call("c1", "write")), reply(message("done"))}
	out = append(out, s)
	for _, p := range []sdk.PermissionMode{sdk.PermissionModeReadOnly, "typo", "", "  DANGER-FULL-ACCESS  ", sdk.PermissionModeWorkspaceWrite} {
		s = baseline("permission-" + fmt.Sprintf("%q", p))
		s.Permission = p
		s.Responses = []response{reply(call("c1", "write")), reply(message("done"))}
		out = append(out, s)
	}
	s = baseline("mode-clamps-explicit-full")
	s.Access = sdk.ToolAccessLevelFull
	s.Snapshot = &mode.TemplateSpec{ToolAccess: "read-only"}
	out = append(out, s)
	s = baseline("mode-turn-limit")
	s.MaxTurns = 0
	s.Snapshot = &mode.TemplateSpec{Constraints: &mode.Constraints{MaxTurns: 1}}
	s.Responses = []response{reply(call("c1", "write"))}
	out = append(out, s)
	for _, op := range []string{"config.permission", "config.directive", "config.guardrails", "config.mode", "session.state", "factory.build", "config.handoff", "session.load"} {
		s = baseline("failure-" + op)
		s.Fail = map[string]int{op: 1}
		out = append(out, s)
	}
	for _, rule := range []sdk.GuardrailRule{{Name: "bad-regex", Type: "tool-input", Regex: "["}, {Name: "bad-action", Type: "tool-input", Regex: "x", Action: "deny"}, {Name: "bad-type", Type: "unknown", Regex: "x"}} {
		s = baseline("compile-" + rule.Name)
		s.Rules = []sdk.GuardrailRule{rule}
		out = append(out, s)
	}
	for _, kind := range []string{"tool-input", "tool-output"} {
		s = approvals("approved-config-" + kind)
		s.Rules = []sdk.GuardrailRule{{Name: "block-secret", Type: kind, Regex: "secret", ToolPattern: "write", Message: "blocked by config"}}
		out = append(out, s)
	}
	s = baseline("default-50-drains-full-page")
	s.Pages = nil
	msgs := []sdk.UserMessage{}
	for i := 1; i <= 50; i++ {
		msgs = append(msgs, sdk.UserMessage{ID: int64(i), Content: fmt.Sprintf("message-%02d", i)})
	}
	s.Pages = []page{{Messages: msgs, Next: sdk.Cursor{MessageID: 50}}, {Messages: []sdk.UserMessage{{ID: 51, Content: "last"}}, Next: sdk.Cursor{MessageID: 51}}}
	out = append(out, s)
	s = baseline("pagination-blank-image-and-verbatim-payload")
	s.MessageLimit = 2
	s.Pages = []page{{Messages: []sdk.UserMessage{{ID: 1, Content: " \t\n"}, {ID: 2, Content: "", Images: []sdk.ImageAttachment{{MediaType: "image/png", Data: "aW1hZ2U=", Detail: "low"}}}}, Next: sdk.Cursor{MessageID: 2, Token: "two"}}, {Messages: []sdk.UserMessage{{ID: 3, Content: "  keep whitespace\nrun_id=abc time=2020-01-01T00:00:00Z  ", Mode: "not-injected"}, {ID: 4, Content: "four"}}, Next: sdk.Cursor{MessageID: 4, Token: "four"}}, {Next: sdk.Cursor{MessageID: 4, Token: "empty-page-token"}}}
	out = append(out, s)
	s = baseline("full-page-stuck-cursor")
	s.MessageLimit = 1
	s.Cursor = sdk.Cursor{MessageID: 1, Token: "same"}
	s.Pages[0].Next = s.Cursor
	out = append(out, s)
	s = baseline("token-only-cursor-advances")
	s.MessageLimit = 1
	s.Cursor = sdk.Cursor{MessageID: 1, Token: "old"}
	s.Pages[0].Next = sdk.Cursor{MessageID: 1, Token: "new"}
	s.Pages = append(s.Pages, page{Next: sdk.Cursor{MessageID: 1, Token: "new"}})
	out = append(out, s)
	s = baseline("cursor-retained-before-later-page-error")
	s.MessageLimit = 1
	s.Pages = append(s.Pages, page{Next: sdk.Cursor{MessageID: 999}})
	s.Fail = map[string]int{"session.load": 2}
	out = append(out, s)
	s = baseline("repeated-run-cursor-no-implicit-history")
	s.Runs = 2
	s.Pages = append(s.Pages, page{Messages: []sdk.UserMessage{{ID: 2, Content: "second input only"}}, Next: sdk.Cursor{MessageID: 2}})
	s.Responses = append(s.Responses, reply(message("second answer")))
	out = append(out, s)
	s = baseline("model-error-discards-response-and-advances-cursor")
	s.Responses[0].Error = "model failure with partial response"
	out = append(out, s)
	no := false
	yes := true
	s = baseline("model-error-discards-prior-runner-turn")
	first := reply(message("partial earlier turn"))
	first.EndTurn = &no
	s.Responses = []response{first, {Error: "later model failure"}}
	out = append(out, s)
	s = baseline("empty-new-items-still-appended")
	s.Responses = []response{{EndTurn: &yes}}
	out = append(out, s)
	for _, op := range []string{"session.append", "trace.final", "status.final"} {
		s = baseline("failure-" + op)
		s.Fail = map[string]int{op: 1}
		out = append(out, s)
	}
	s = approvals("batch-approved-before-model-resume")
	out = append(out, s)
	s = approvals("batch-no-gate-denied-without-resume")
	s.Gate = false
	out = append(out, s)
	s = approvals("gate-denials-explicit-and-blank-reason")
	s.Decisions = []decision{{Reason: "  host says no  "}, {Reason: " \n "}}
	out = append(out, s)
	s = approvals("approved-pause-still-resolves-entire-batch")
	s.FactoryTools[0].Pause = true
	out = append(out, s)
	s = approvals("gate-failure-after-first-approved-effect")
	s.Fail = map[string]int{"gate.approve": 2}
	out = append(out, s)
	s = approvals("approval-append-failure-precedes-gate-error")
	s.Fail = map[string]int{"gate.approve": 2, "session.append": 2}
	out = append(out, s)
	s = approvals("approval-append-failure-prevents-resume")
	s.Fail = map[string]int{"session.append": 2}
	out = append(out, s)
	s = approvals("initial-append-failure-prevents-approvals")
	s.Fail = map[string]int{"session.append": 1}
	out = append(out, s)
	s = approvals("no-gate-denial-append-failure")
	s.Gate = false
	s.Fail = map[string]int{"session.append": 2}
	out = append(out, s)
	s = approvals("model-resume-error-discards-combined-result-not-effects")
	s.Responses[1].Error = "resume model failure"
	out = append(out, s)
	for _, limit := range []int{1, 0} {
		s = approvals(fmt.Sprintf("max-resumes-%d", limit))
		s.MaxResumes = limit
		s.Responses = nil
		s.Decisions = nil
		effective := limit
		if effective <= 0 {
			effective = 12
		}
		for i := 0; i <= effective; i++ {
			s.Responses = append(s.Responses, reply(call(fmt.Sprintf("c%d", i), "write")))
			s.Decisions = append(s.Decisions, decision{Approved: true})
		}
		out = append(out, s)
	}
	return out
}
