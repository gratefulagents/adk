package main

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type observedEvent struct {
	Op   string
	Args json.RawMessage
}
type observedResult struct {
	NewItems      []sdk.LLMRunItemSnapshot `json:"new_items"`
	FinalHistory  []sdk.LLMRunItemSnapshot `json:"final_history"`
	Interrupted   bool                     `json:"interrupted"`
	Interruption  *sdk.Interruption        `json:"interruption"`
	Interruptions []*sdk.Interruption      `json:"interruptions"`
	Usage         sdk.Usage                `json:"usage"`
}
type observedRun struct {
	Calls        []observedEvent
	CursorBefore sdk.Cursor `json:"cursor_before"`
	CursorAfter  sdk.Cursor `json:"cursor_after"`
	Result       *observedResult
	Error        *struct{ Category, Message string }
}

func observe(t *testing.T, name string) []observedRun {
	t.Helper()
	for _, s := range scenarios() {
		if s.Name == name {
			b, err := json.Marshal(execute(s))
			if err != nil {
				t.Fatal(err)
			}
			var c struct{ Runs []observedRun }
			if err = json.Unmarshal(b, &c); err != nil {
				t.Fatal(err)
			}
			return c.Runs
		}
	}
	t.Fatalf("missing scenario %s", name)
	return nil
}
func events(r observedRun, op string) []observedEvent {
	out := []observedEvent{}
	for _, e := range r.Calls {
		if e.Op == op {
			out = append(out, e)
		}
	}
	return out
}
func decode[T any](t *testing.T, e observedEvent) T {
	t.Helper()
	var out T
	if err := json.Unmarshal(e.Args, &out); err != nil {
		t.Fatal(err)
	}
	return out
}
func requireCount(t *testing.T, r observedRun, op string, n int) {
	t.Helper()
	if got := len(events(r, op)); got != n {
		t.Fatalf("%s count=%d, want %d", op, got, n)
	}
}
func requireError(t *testing.T, r observedRun, category string) {
	t.Helper()
	if r.Error == nil || r.Error.Category != category {
		t.Fatalf("error=%+v, want %s", r.Error, category)
	}
}
func TestPreparation(t *testing.T) {
	r := observe(t, "preparation-order-and-handoff")[0]
	ops := []string{}
	for _, e := range r.Calls {
		ops = append(ops, e.Op)
	}
	want := []string{"config.permission", "config.directive", "config.guardrails", "config.mode", "session.state", "factory.build", "config.handoff", "session.load", "runner.start", "model.response", "session.append", "trace.final", "status.final"}
	if !reflect.DeepEqual(ops, want) {
		t.Fatalf("calls=%v", ops)
	}
	cfg := decode[map[string]any](t, events(r, "runner.start")[0])
	if cfg["access"] != "read-only" || cfg["max_turns"] != float64(8) || cfg["subagent_max_turns"] != float64(4) || !strings.Contains(cfg["working_state_context"].(string), "Current step: inspect") {
		t.Fatalf("config=%v", cfg)
	}
	req := decode[struct {
		Input        []sdk.LLMRunItemSnapshot
		Instructions string
		Tools        []sdk.LLMToolSnapshot
	}](t, events(r, "model.response")[0])
	if len(req.Input) != 3 || req.Input[0].AgentName != "prior-agent" || len(req.Tools) != 1 || req.Tools[0].Name != "read" {
		t.Fatalf("request=%+v", req)
	}
	if strings.Contains(req.Instructions, "Durable Working State") || strings.Contains(req.Instructions, "NOT consumed") {
		t.Fatal("unexpected prompt injection")
	}
	r = observe(t, "explicit-config-precedence")[0]
	cfg = decode[map[string]any](t, events(r, "runner.start")[0])
	if cfg["access"] != "full" || cfg["max_turns"] != float64(2) || cfg["subagent_max_turns"] != float64(5) || cfg["working_state_context"] != "retained host state" {
		t.Fatalf("explicit config lost: %v", cfg)
	}
	requireCount(t, r, "tool.execute", 1)
	requireCount(t, r, "gate.approve", 0)
}
func TestPreparationShortCircuits(t *testing.T) {
	cases := map[string]string{"config.permission": "load permission mode", "config.directive": "load mode directive", "config.guardrails": "load guardrail rules", "config.mode": "load mode snapshot", "session.state": "load working state", "factory.build": "build platform tools", "config.handoff": "load handoff history", "session.load": "load session messages"}
	for op, category := range cases {
		t.Run(op, func(t *testing.T) {
			r := observe(t, "failure-"+op)[0]
			requireError(t, r, category)
			if r.Result != nil || r.Calls[len(r.Calls)-1].Op != op || r.CursorAfter != (sdk.Cursor{}) {
				t.Fatalf("unexpected partial preparation: %+v", r)
			}
		})
	}
	for _, kind := range []string{"regex", "action", "type"} {
		t.Run(kind, func(t *testing.T) {
			r := observe(t, "compile-bad-"+kind)[0]
			requireError(t, r, "compile guardrail rules")
			requireCount(t, r, "config.mode", 0)
			requireCount(t, r, "model.response", 0)
		})
	}
}
func TestPagingAndRepeat(t *testing.T) {
	r := observe(t, "default-50-drains-full-page")[0]
	requireCount(t, r, "session.load", 2)
	for _, e := range events(r, "session.load") {
		if decode[struct{ Limit int }](t, e).Limit != 50 {
			t.Fatal("default limit")
		}
	}
	req := decode[struct{ Input []sdk.LLMRunItemSnapshot }](t, events(r, "model.response")[0])
	if len(req.Input) != 51 {
		t.Fatal("full page lost")
	}
	r = observe(t, "pagination-blank-image-and-verbatim-payload")[0]
	requireCount(t, r, "session.load", 3)
	req = decode[struct{ Input []sdk.LLMRunItemSnapshot }](t, events(r, "model.response")[0])
	if len(req.Input) != 3 || len(req.Input[0].MessageImages) != 1 || req.Input[1].MessageText != "  keep whitespace\nrun_id=abc time=2020-01-01T00:00:00Z  " {
		t.Fatalf("payload changed: %+v", req)
	}
	requireCount(t, observe(t, "full-page-stuck-cursor")[0], "session.load", 1)
	requireCount(t, observe(t, "token-only-cursor-advances")[0], "session.load", 2)
	r = observe(t, "cursor-retained-before-later-page-error")[0]
	requireError(t, r, "load session messages")
	if r.CursorAfter.MessageID != 1 || r.Result != nil {
		t.Fatal("failed page must not adopt returned cursor")
	}
	rs := observe(t, "repeated-run-cursor-no-implicit-history")
	if rs[1].CursorBefore != rs[0].CursorAfter {
		t.Fatal("cursor reset")
	}
	req = decode[struct{ Input []sdk.LLMRunItemSnapshot }](t, events(rs[1], "model.response")[0])
	if len(req.Input) != 1 || req.Input[0].MessageText != "second input only" {
		t.Fatalf("implicit history: %+v", req)
	}
}
func TestRunnerErrorsDiscardPartial(t *testing.T) {
	for _, name := range []string{"model-error-discards-response-and-advances-cursor", "model-error-discards-prior-runner-turn", "model-resume-error-discards-combined-result-not-effects", "mode-turn-limit"} {
		t.Run(name, func(t *testing.T) {
			r := observe(t, name)[0]
			requireError(t, r, "runner")
			if r.Result != nil || r.CursorAfter.MessageID != 1 {
				t.Fatal("must discard partial result, not cursor")
			}
			requireCount(t, r, "trace.final", 0)
			requireCount(t, r, "status.final", 0)
			if name == "model-resume-error-discards-combined-result-not-effects" {
				requireCount(t, r, "session.append", 2)
				requireCount(t, r, "tool.execute", 2)
			} else {
				requireCount(t, r, "session.append", 0)
			}
		})
	}
}
func TestPersistenceAndFinalization(t *testing.T) {
	r := observe(t, "empty-new-items-still-appended")[0]
	requireCount(t, r, "session.append", 1)
	if decode[struct{ Count int }](t, events(r, "session.append")[0]).Count != 0 || r.Result == nil || r.Error != nil {
		t.Fatal("empty append contract")
	}
	for _, tc := range []struct{ Name, Category, Last string }{
		{"failure-session.append", "append run items", "session.append"},
		{"failure-trace.final", "finalize trace", "trace.final"},
		{"failure-status.final", "publish final result", "status.final"},
		{"initial-append-failure-prevents-approvals", "append run items", "session.append"},
		{"no-gate-denial-append-failure", "append denied approval items", "session.append"},
		{"approval-append-failure-prevents-resume", "append approval items", "session.append"},
		{"approval-append-failure-precedes-gate-error", "append approval items", "session.append"},
	} {
		t.Run(tc.Name, func(t *testing.T) {
			r := observe(t, tc.Name)[0]
			requireError(t, r, tc.Category)
			if r.Result == nil || r.Calls[len(r.Calls)-1].Op != tc.Last {
				t.Fatal("failure must retain combined result and stop")
			}
		})
	}
}
func TestApprovalBatchAndPause(t *testing.T) {
	for _, name := range []string{"batch-approved-before-model-resume", "approved-pause-still-resolves-entire-batch", "gate-failure-after-first-approved-effect"} {
		t.Run(name, func(t *testing.T) {
			r := observe(t, name)[0]
			requireCount(t, r, "gate.approve", 2)
			n := 2
			if name == "gate-failure-after-first-approved-effect" {
				n = 1
				requireError(t, r, "approve tool")
				requireCount(t, r, "trace.final", 0)
			}
			requireCount(t, r, "tool.execute", n)
			appends := events(r, "session.append")
			batch := decode[struct{ Items []sdk.LLMRunItemSnapshot }](t, appends[1])
			if len(batch.Items) != 2*n {
				t.Fatalf("batch=%+v", batch)
			}
			for i := 0; i < n; i++ {
				if batch.Items[2*i].Type != "tool_approval" || !batch.Items[2*i].ToolApproval.Approved || batch.Items[2*i+1].Type != "tool_output" {
					t.Fatalf("unpaired generated batch: %+v", batch)
				}
			}
			if name == "batch-approved-before-model-resume" {
				requireCount(t, r, "model.response", 2)
				req := decode[struct{ Input []sdk.LLMRunItemSnapshot }](t, events(r, "model.response")[1])
				for _, id := range []string{"c1", "c2"} {
					found := false
					for _, item := range req.Input {
						if item.ToolOutput != nil && item.ToolOutput.CallID == id {
							found = true
						}
					}
					if !found {
						t.Fatal("resume before paired output")
					}
				}
				if r.Result.Usage.Requests != 2 {
					t.Fatal("usage not accumulated")
				}
			} else {
				requireCount(t, r, "model.response", 1)
			}
			if name == "approved-pause-still-resolves-entire-batch" {
				if r.Result.Interrupted || r.Result.Interruption != nil || len(r.Result.Interruptions) != 0 {
					t.Fatal("pause leaks pending approvals")
				}
				last := r.Result.FinalHistory[len(r.Result.FinalHistory)-1]
				if last.ToolOutput == nil || last.ToolOutput.CallID != "c2" {
					t.Fatal("pause history missing generated batch")
				}
			}
		})
	}
}
func TestNoGateAndDenials(t *testing.T) {
	r := observe(t, "batch-no-gate-denied-without-resume")[0]
	requireCount(t, r, "model.response", 1)
	requireCount(t, r, "gate.approve", 0)
	requireCount(t, r, "tool.execute", 0)
	batch := decode[struct{ Items []sdk.LLMRunItemSnapshot }](t, events(r, "session.append")[1])
	if len(batch.Items) != 4 || !r.Result.Interrupted {
		t.Fatal("no-gate outcome changed")
	}
	for i := 0; i < 4; i += 2 {
		if batch.Items[i].ToolApproval.Approved || !batch.Items[i+1].ToolOutput.IsError {
			t.Fatal("no-gate must deny and pair")
		}
	}
	r = observe(t, "gate-denials-explicit-and-blank-reason")[0]
	requireCount(t, r, "model.response", 2)
	requireCount(t, r, "tool.execute", 0)
	batch = decode[struct{ Items []sdk.LLMRunItemSnapshot }](t, events(r, "session.append")[1])
	if batch.Items[1].ToolOutput.Content != "host says no" || batch.Items[3].ToolOutput.Content != "tool call denied by host approval gate" {
		t.Fatal("denial reason handling")
	}
}
func TestResumeLimits(t *testing.T) {
	for _, tc := range []struct {
		Name  string
		Limit int
	}{{"max-resumes-1", 1}, {"max-resumes-0", 12}} {
		t.Run(tc.Name, func(t *testing.T) {
			r := observe(t, tc.Name)[0]
			requireError(t, r, "too many chat loop resumes")
			requireCount(t, r, "gate.approve", tc.Limit)
			requireCount(t, r, "model.response", tc.Limit+1)
			requireCount(t, r, "session.append", 2*tc.Limit+1)
			requireCount(t, r, "trace.final", 0)
			if r.Result == nil || !r.Result.Interrupted {
				t.Fatal("limit must return interrupted combined result")
			}
		})
	}
}
func TestApprovedGuardrails(t *testing.T) {
	input := observe(t, "approved-config-tool-input")[0]
	output := observe(t, "approved-config-tool-output")[0]
	requireCount(t, input, "tool.execute", 1)
	requireCount(t, output, "tool.execute", 2)
	for _, r := range []observedRun{input, output} {
		batch := decode[struct{ Items []sdk.LLMRunItemSnapshot }](t, events(r, "session.append")[1])
		if !batch.Items[1].ToolOutput.IsError || !strings.Contains(batch.Items[1].ToolOutput.Content, "guardrail") {
			t.Fatal("approved execution skipped guardrails")
		}
	}
}

func conversationCaseNamed(t *testing.T, fixture object, group, name string) conversationCase {
	t.Helper()
	for _, c := range fixture[group].([]conversationCase) {
		if c.Name == name {
			return c
		}
	}
	t.Fatalf("missing %s/%s", group, name)
	return conversationCase{}
}

func TestConversationFixtureShapeAndDeterminism(t *testing.T) {
	first := conversationFixture()
	counts := map[string]int{"build_conversation_tail": 11, "build_working_state_context": 7, "derive_working_state_goal": 15, "truncate_context_text": 11, "build_assistant_turn_summary": 6, "summarize_turn_tool_calls": 6, "select_next_user_message": 12, "collect_immediate_run_items": 12, "result_helpers": 13}
	for group, count := range counts {
		cases := first[group].([]conversationCase)
		if len(cases) != count {
			t.Fatalf("%s cases=%d, want %d", group, len(cases), count)
		}
		seen := map[string]bool{}
		for _, c := range cases {
			if seen[c.Name] || c.Name == "" || !json.Valid(c.Input) || c.Output == nil {
				t.Fatalf("invalid or duplicate case %s/%s", group, c.Name)
			}
			seen[c.Name] = true
		}
	}
	a, err := json.Marshal(first)
	if err != nil {
		t.Fatal(err)
	}
	b, err := json.Marshal(conversationFixture())
	if err != nil || string(a) != string(b) {
		t.Fatalf("conversation generation is not repeatable: %v", err)
	}
}

func TestConversationTailAndTextBoundaries(t *testing.T) {
	f := conversationFixture()
	get := func(group, name string) any { return conversationCaseNamed(t, f, group, name).Output }
	tail := get("build_conversation_tail", "floor-exclude-roles-images").(itemsOutput)
	if tail.Count != 5 || tail.Items[0].AgentName != "assistant-summary" || tail.Items[1].AgentName != "system-summary" || tail.Items[2].AgentName != "" || len(tail.Items[2].MessageImages) != 2 || tail.Items[3].AgentName != "" || tail.Items[4].AgentName != "" {
		t.Fatalf("tail provenance/images: %+v", tail)
	}
	for _, name := range []string{"limit-0", "limit--2"} {
		if out := get("build_conversation_tail", name).(itemsOutput); out.Count != 8 || out.Items[0].MessageText != "message-4" {
			t.Fatalf("default tail limit: %+v", out)
		}
	}
	if out := get("build_conversation_tail", "unicode-1200-rune-cap").(itemsOutput); len([]rune(out.Items[0].MessageText)) != 1203 {
		t.Fatal("tail must truncate runes, not bytes")
	}
	if got := get("truncate_context_text", "text-02").(textOutput).Text; got != "a  b\t c\r d" {
		t.Fatalf("unexpected whitespace normalization: %q", got)
	}
	if got := get("truncate_context_text", "text-04").(textOutput).Text; got != "界..." {
		t.Fatalf("rune truncation: %q", got)
	}
	if got := get("truncate_context_text", "text-08").(textOutput).Text; got != "e..." {
		t.Fatalf("runes are not grapheme clusters: %q", got)
	}
	state := get("build_working_state_context", "all-fields-last-four").(textOutput).Text
	if strings.Contains(state, "discard-me") || strings.Contains(state, "not-in-context") || !strings.Contains(state, "Mode:  plan\nraw ") || !strings.Contains(state, "- two lines\n- \n- five") {
		t.Fatalf("state context: %q", state)
	}
	if got := get("build_working_state_context", "same-goal-direction-omitted").(textOutput).Text; strings.Contains(got, "Latest user direction:") {
		t.Fatal("duplicate user direction")
	}
	if got := get("build_working_state_context", "raw-equality-before-trimming").(textOutput).Text; !strings.Contains(got, "Latest user direction: same") {
		t.Fatal("state equality must precede trimming")
	}
	if got := get("derive_working_state_goal", "reply-08").(textOutput).Text; got != "original" {
		t.Fatalf("approval shorthand: %q", got)
	}
	if got := get("derive_working_state_goal", "reply-13").(textOutput).Text; got != "approve : spaced colon" {
		t.Fatalf("overbroad shorthand: %q", got)
	}
}

func TestConversationSummaryUniquenessAndSort(t *testing.T) {
	f := conversationFixture()
	out := conversationCaseNamed(t, f, "build_assistant_turn_summary", "unique-assistant-first-two-tools-issues").Output.(textOutput).Text
	want := "first answer\nsecond\nTools: alpha x 2, beta x 2,  x 1, delta x 1\nIssues: failure one | second failure"
	if out != want {
		t.Fatalf("summary=%q, want %q", out, want)
	}
	out = conversationCaseNamed(t, f, "build_assistant_turn_summary", "unique-successes-without-assistant").Output.(textOutput).Text
	if out != "Key results: result one | two\nIssues: bad" {
		t.Fatalf("success summary=%q", out)
	}
	out = conversationCaseNamed(t, f, "build_assistant_turn_summary", "dedup-after-220-rune-truncation").Output.(textOutput).Text
	if out != strings.Repeat("界", 220)+"...\nsecond unique" {
		t.Fatal("summary uniqueness must follow truncation")
	}
	for _, name := range []string{"frequency-ties-limit-0", "frequency-ties-limit--1"} {
		got := conversationCaseNamed(t, f, "summarize_turn_tool_calls", name).Output.(toolSummaryOutput).Summaries
		if !reflect.DeepEqual(got, []string{"alpha x 2", "beta x 2", " x 1", "delta x 1", "gamma x 1", "zeta x 1"}) {
			t.Fatalf("tool sorting/limit: %v", got)
		}
	}
}

func TestConversationConsumedMutationAndCursorSafety(t *testing.T) {
	f := conversationFixture()
	for _, name := range []string{"queue-before-immediates", "image-only-queue-barrier"} {
		out := conversationCaseNamed(t, f, "collect_immediate_run_items", name).Output.(immediateOutput)
		selected := conversationCaseNamed(t, f, "select_next_user_message", name).Output.(selectionOutput)
		if out.Cursor != 0 || selected.SkipCursor != 0 || !selected.Immediate || len(selected.ConsumedImmediateAfter) != 0 || out.Count == 0 {
			t.Fatalf("queued input dropped or select mutated state: %+v / %+v", out, selected)
		}
	}
	c := conversationCaseNamed(t, f, "collect_immediate_run_items", "consumed-prefix-queue-barrier")
	var in queueInput
	if err := json.Unmarshal(c.Input, &in); err != nil {
		t.Fatal(err)
	}
	out := c.Output.(immediateOutput)
	if !reflect.DeepEqual(in.ConsumedImmediate, []int64{2, 4, 99}) || !reflect.DeepEqual(out.ConsumedImmediateAfter, []int64{2, 4, 5, 99}) || out.Cursor != 2 {
		t.Fatalf("pre/post mutation evidence lost: %+v / %+v", in, out)
	}
	for _, group := range []string{"select_next_user_message", "collect_immediate_run_items"} {
		c := conversationCaseNamed(t, f, group, "out-of-order-prefix-not-max-id")
		cursor := int64(0)
		if group == "select_next_user_message" {
			cursor = c.Output.(selectionOutput).SkipCursor
		} else {
			cursor = c.Output.(immediateOutput).Cursor
		}
		if cursor != 7 {
			t.Fatalf("%s cursor=%d; SDK follows slice order, not max ID", group, cursor)
		}
	}
	out = conversationCaseNamed(t, f, "collect_immediate_run_items", "out-of-order-immediate-slice-order").Output.(immediateOutput)
	if out.Cursor != 60 || out.Count != 3 || out.Items[0].MessageText != "first" || out.Items[1].MessageText != "second" {
		t.Fatalf("unexpected sorting: %+v", out)
	}
	out = conversationCaseNamed(t, f, "collect_immediate_run_items", "duplicate-immediate-id").Output.(immediateOutput)
	if out.Count != 1 || !reflect.DeepEqual(out.ConsumedImmediateAfter, []int64{5}) {
		t.Fatalf("duplicate consumption: %+v", out)
	}
}

func TestResultHelpersPreserveLegacyContradictions(t *testing.T) {
	f := conversationFixture()
	for _, c := range f["result_helpers"].([]conversationCase) {
		var in resultHelperInput
		if err := json.Unmarshal(c.Input, &in); err != nil {
			t.Fatal(err)
		}
		out := c.Output.(resultHelperOutput)
		if out.InputCount != len(in.NewItems) || !reflect.DeepEqual(out.InputList, sdk.SnapshotRunItems(restore(in.NewItems))) {
			t.Fatalf("ToInputList used history instead of NewItems: %s", c.Name)
		}
		if strings.HasPrefix(c.Name, "final-output-") && c.Name != "final-output-string" && out.FinalText != "" {
			t.Fatalf("FinalText coerced nonstring: %s", c.Name)
		}
		if in.SourceOnly != strings.HasPrefix(in.LegacyShape, "contradictory-") {
			t.Fatalf("unmarked source contradiction: %s", c.Name)
		}
	}
	c := conversationCaseNamed(t, f, "result_helpers", "final-output-missing")
	if strings.Contains(string(c.Input), "final_output") {
		t.Fatal("missing must remain distinct from explicit null")
	}
	if out := conversationCaseNamed(t, f, "result_helpers", "final-output-string").Output.(resultHelperOutput); out.FinalText != "  final\n界🙂  " {
		t.Fatal("FinalText must preserve string verbatim")
	}
	out := conversationCaseNamed(t, f, "result_helpers", "plural-without-singular").Output.(resultHelperOutput)
	if out.IsInterrupted || len(out.AllInterruptions) != 2 {
		t.Fatalf("source contradiction flattened: %+v", out)
	}
	out = conversationCaseNamed(t, f, "result_helpers", "singular-disagrees-with-plural").Output.(resultHelperOutput)
	if !out.IsInterrupted || len(out.AllInterruptions) != 1 || out.AllInterruptions[0].ToolCallID != "c2" {
		t.Fatalf("plural must win AllInterruptions: %+v", out)
	}
}
