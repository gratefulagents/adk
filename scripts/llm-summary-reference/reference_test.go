// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"
	"time"
	"unicode/utf8"
)

type llmText struct {
	Prefix string `json:"prefix,omitempty"`
	Repeat string `json:"repeat,omitempty"`
	Count  int    `json:"count,omitempty"`
	Suffix string `json:"suffix,omitempty"`
}

func (s llmText) expand() string { return s.Prefix + strings.Repeat(s.Repeat, s.Count) + s.Suffix }

type llmItem struct {
	Kind    string  `json:"kind"`
	Agent   *string `json:"agent,omitempty"`
	Missing bool    `json:"missing,omitempty"`
	Text    llmText `json:"text"`
	Name    string  `json:"name,omitempty"`
	ID      string  `json:"id,omitempty"`
	Input   llmText `json:"input"`
	IsError bool    `json:"isError,omitempty"`
}

func llmMessage(agent, text string) llmItem {
	i := llmItem{Kind: "message", Text: llmText{Prefix: text}}
	if agent != "" {
		i.Agent = &agent
	}
	return i
}

func llmItems(specs []llmItem) []RunItem {
	if specs == nil {
		return nil
	}
	items := make([]RunItem, 0, len(specs))
	for _, s := range specs {
		i := RunItem{}
		if s.Agent != nil {
			i.Agent = &Agent{Name: *s.Agent}
		}
		switch s.Kind {
		case "message":
			i.Type = RunItemMessage
			i.Message = &MessageOutput{Text: s.Text.expand()}
		case "reasoning":
			i.Type = RunItemReasoning
			i.Reasoning = &ReasoningData{Text: s.Text.expand()}
		case "toolCall":
			i.Type = RunItemToolCall
			i.ToolCall = &ToolCallData{ID: s.ID, Name: s.Name, Input: []byte(s.Input.expand())}
		case "toolOutput":
			i.Type = RunItemToolOutput
			i.ToolOutput = &ToolOutputData{CallID: s.ID, Content: s.Text.expand(), IsError: s.IsError}
		case "handoffCall":
			i.Type = RunItemHandoffCall
			i.HandoffCall = &HandoffCallData{FromAgent: "a", ToAgent: "b"}
		case "handoffOutput":
			i.Type = RunItemHandoffOutput
			i.HandoffOutput = &HandoffOutputData{FromAgent: "a", ToAgent: "b"}
		case "approval":
			i.Type = RunItemToolApproval
			i.ToolApproval = &ToolApprovalData{ToolName: s.Name}
		case "compaction":
			i.Type = RunItemCompaction
			i.Compaction = &CompactionData{EncryptedContent: s.Text.expand()}
		case "unknown":
			i.Type = RunItemType(99)
		default:
			panic(s.Kind)
		}
		if s.Missing {
			i = RunItem{Type: i.Type, Agent: i.Agent}
		}
		items = append(items, i)
	}
	return items
}

func llmStringObservation(s string) map[string]any {
	hash := sha256.Sum256([]byte(s))
	o := map[string]any{"bytes": len(s), "runes": utf8.RuneCountInString(s), "validUTF8": utf8.ValidString(s), "sha256": hex.EncodeToString(hash[:])}
	if len(s) <= 18000 {
		o["text"] = s
	} else {
		r := []rune(s)
		o["head"] = string(r[:128])
		o["tail"] = string(r[len(r)-128:])
	}
	return o
}

func llmItemObservations(items []RunItem) []map[string]any {
	if items == nil {
		return nil
	}
	out := make([]map[string]any, 0, len(items))
	for _, i := range items {
		o := map[string]any{"type": int(i.Type), "agent": nil}
		if i.Agent != nil {
			o["agent"] = i.Agent.Name
		}
		if i.Message != nil {
			o["message"] = llmStringObservation(i.Message.Text)
		}
		if i.Reasoning != nil {
			o["reasoning"] = llmStringObservation(i.Reasoning.Text)
		}
		if i.ToolCall != nil {
			o["toolCall"] = map[string]any{"id": i.ToolCall.ID, "name": i.ToolCall.Name, "input": llmStringObservation(string(i.ToolCall.Input))}
		}
		if i.ToolOutput != nil {
			o["toolOutput"] = map[string]any{"callID": i.ToolOutput.CallID, "isError": i.ToolOutput.IsError, "content": llmStringObservation(i.ToolOutput.Content)}
		}
		out = append(out, o)
	}
	return out
}

func llmFlattenObservations() []map[string]any {
	cases := []struct {
		name   string
		items  []llmItem
		budget int
	}{
		{"roles-and-skips", []llmItem{
			llmMessage("", " \u2003task\nline\u00a0 "), llmMessage("assistant", " answer "),
			{Kind: "message", Agent: new(string), Text: llmText{Prefix: "unnamed assistant"}},
			{Kind: "reasoning", Text: llmText{Prefix: " thinking\nnext "}},
			{Kind: "toolCall", ID: "c1", Name: "read_file", Input: llmText{Prefix: ` {"path":"a.go"} `}},
			{Kind: "toolOutput", ID: "c1", Text: llmText{Prefix: " result "}},
			{Kind: "toolOutput", ID: "c2", IsError: true, Text: llmText{Prefix: " failed "}},
			{Kind: "toolCall", Name: "empty"}, {Kind: "toolOutput"},
			llmMessage("", "\n\t "), {Kind: "reasoning", Text: llmText{Prefix: "\u2003"}},
			{Kind: "message", Missing: true}, {Kind: "reasoning", Missing: true},
			{Kind: "toolCall", Missing: true}, {Kind: "toolOutput", Missing: true},
			{Kind: "handoffCall"}, {Kind: "handoffOutput"}, {Kind: "approval", Name: "approve"},
			{Kind: "compaction", Text: llmText{Prefix: "opaque"}}, {Kind: "unknown"},
		}, 0},
		{"head-tail", []llmItem{llmMessage("", "HEAD-TASK"), llmMessage("a", strings.Repeat("filler-", 20)), llmMessage("a", "TAIL-STATE")}, 45},
		{"utf8-head-tail", []llmItem{llmMessage("", "界🙂é界🙂é界🙂é界🙂é界🙂é界🙂é")}, 29},
		{"empty", nil, 10},
	}
	for _, budget := range []int{-1, 0, 1, 2, 3, 9, 10, 11, 12, 13} {
		cases = append(cases, struct {
			name   string
			items  []llmItem
			budget int
		}{fmt.Sprintf("whole-budget-%d", budget), []llmItem{llmMessage("", "abc")}, budget})
	}
	for _, c := range []struct {
		kind    string
		limit   int
		isError bool
	}{
		{"message", 2000, false}, {"reasoning", 1500, false}, {"toolCall", 300, false}, {"toolOutput", 700, false}, {"toolOutput", 1000, true},
	} {
		for _, delta := range []int{-1, 0, 1} {
			s := llmItem{Kind: c.kind, Name: "tool", IsError: c.isError, Text: llmText{Repeat: "x", Count: c.limit + delta}}
			if c.kind == "toolCall" {
				s.Input = s.Text
				s.Text = llmText{}
			}
			cases = append(cases, struct {
				name   string
				items  []llmItem
				budget int
			}{fmt.Sprintf("%s-error-%v-limit-%d", c.kind, c.isError, delta), []llmItem{s}, 0})
		}
		s := llmItem{Kind: c.kind, Name: "tool", IsError: c.isError, Text: llmText{Repeat: "x", Count: c.limit - 1, Suffix: "🙂TAIL"}}
		if c.kind == "toolCall" {
			s.Input = s.Text
			s.Text = llmText{}
		}
		cases = append(cases, struct {
			name   string
			items  []llmItem
			budget int
		}{fmt.Sprintf("%s-error-%v-utf8", c.kind, c.isError), []llmItem{s}, 0})
	}
	for _, c := range []struct {
		name, agent, prefix string
		size                int
	}{
		{"retained", "context-summary", "[COMPACTED HISTORY SUMMARY]\n", 5000},
		{"trimmed-marker", "context-summary", " \n[COMPACTED HISTORY SUMMARY]\n", 5000},
		{"wrong-agent", "assistant", "[COMPACTED HISTORY SUMMARY]\n", 5000},
		{"user-marker", "", "[COMPACTED HISTORY SUMMARY]\n", 5000},
		{"missing-marker", "context-summary", "not a marker\n", 5000},
		{"case-sensitive", "context-summary", "[compacted history summary]\n", 5000},
		{"at-limit", "context-summary", "[COMPACTED HISTORY SUMMARY]\n", 16000},
		{"over-limit", "context-summary", "[COMPACTED HISTORY SUMMARY]\n", 16001},
		{"utf8-limit", "context-summary", "[COMPACTED HISTORY SUMMARY]\n", 15999},
	} {
		s := llmMessage(c.agent, "")
		s.Text = llmText{Prefix: c.prefix, Repeat: "p", Count: c.size - len(c.prefix)}
		if c.name == "utf8-limit" {
			s.Text.Suffix = "🙂TAIL"
		}
		cases = append(cases, struct {
			name   string
			items  []llmItem
			budget int
		}{"prior-summary-" + c.name, []llmItem{s}, 0})
	}
	var large []llmItem
	for i := 0; i < 250; i++ {
		s := llmMessage("a", "")
		s.Text = llmText{Prefix: fmt.Sprintf("segment-%03d:", i), Repeat: "界", Count: 400}
		large = append(large, s)
	}
	cases = append(cases, struct {
		name   string
		items  []llmItem
		budget int
	}{"default-240000-byte-budget", large, llmSummaryTranscriptCharBudget})
	out := []map[string]any{}
	for _, c := range cases {
		out = append(out, map[string]any{"name": c.name, "items": c.items, "maxBytes": c.budget, "output": llmStringObservation(flattenRunItemsForSummary(llmItems(c.items), c.budget))})
	}
	return out
}

func llmTruncateObservations() []map[string]any {
	out := []map[string]any{}
	for _, s := range []string{"", " \u2003é界🙂z\u00a0 ", "a\u0301b", "abc"} {
		for n := 0; n <= 12; n++ {
			out = append(out, map[string]any{"input": s, "maxBytes": n, "output": llmStringObservation(truncateForTranscript(s, n))})
		}
	}
	return out
}

type llmOracleModel struct {
	Model
	t            *testing.T
	response     *ModelResponse
	err          error
	contextError bool
	timeout      time.Duration
	parent       context.Context
	calls        []map[string]any
}

func (m *llmOracleModel) GetResponse(ctx context.Context, req ModelRequest) (*ModelResponse, error) {
	deadline, ok := ctx.Deadline()
	bound := m.timeout
	if bound <= 0 {
		bound = llmSummaryCallTimeout
	}
	parentDeadline, parentOK := m.parent.Deadline()
	inherited := parentOK && parentDeadline.Before(time.Now().Add(bound))
	valid := ok && deadline.After(time.Now().Add(bound-time.Second)) && !deadline.After(time.Now().Add(bound))
	if inherited {
		valid = ok && deadline.Equal(parentDeadline)
	}
	if !valid {
		m.t.Fatal("summary deadline did not match configured/default/parent bound")
	}
	if !reflect.DeepEqual(req.Settings, ModelSettings{MaxTokens: 2048, ReasoningEffort: "low"}) {
		m.t.Fatalf("unexpected summary settings: %+v", req.Settings)
	}
	m.calls = append(m.calls, map[string]any{
		"model": req.Model, "promptCacheKey": req.PromptCacheKey, "instructions": req.Instructions,
		"input": llmItemObservations(req.Input), "toolsNil": req.Tools == nil, "toolCount": len(req.Tools),
		"settings": req.Settings, "outputSchema": req.OutputSchema, "compactionThreshold": req.CompactionThreshold,
		"deadlinePresent": ok, "deadlineMatchesPolicy": valid, "inheritsParentDeadline": inherited, "configuredOrDefaultTimeoutNanos": int64(bound),
	})
	if m.contextError {
		return nil, ctx.Err()
	}
	return m.response, m.err
}

func llmSummaryObservations(t *testing.T) []map[string]any {
	usage := Usage{Requests: 3, InputTokens: 501, OutputTokens: 43, CacheReadTokens: 71, CacheCreateTokens: 17}
	removed := []llmItem{llmMessage("", " fix a.go "), {Kind: "reasoning", Text: llmText{Prefix: " preserve constraints "}}}
	cases := []struct {
		name    string
		items   []llmItem
		err     string
		timeout time.Duration
		parent  string
	}{
		{"join-filter-strip", []llmItem{{Kind: "reasoning", Text: llmText{Prefix: "ignore"}}, llmMessage("a", " \n[COMPACTED HISTORY SUMMARY]\n First \n "), {Kind: "message", Missing: true}, llmMessage("", "\u2003"), {Kind: "toolOutput", Text: llmText{Prefix: "ignore"}}, llmMessage("", " Second \t")}, "", 0, ""},
		{"single-prefix-only", []llmItem{llmMessage("a", "[COMPACTED HISTORY SUMMARY][COMPACTED HISTORY SUMMARY] next")}, "", 17 * time.Second, ""},
		{"marker-interior", []llmItem{llmMessage("a", "Intro [COMPACTED HISTORY SUMMARY] tail")}, "", -time.Second, ""},
		{"marker-case-sensitive", []llmItem{llmMessage("a", "[compacted history summary] next")}, "", 0, ""},
		{"marker-only", []llmItem{llmMessage("a", " [COMPACTED HISTORY SUMMARY] \n")}, "", 0, ""},
		{"whitespace-only", []llmItem{llmMessage("a", " \u2003\n")}, "", 0, ""},
		{"no-message", []llmItem{{Kind: "toolCall", Name: "ignored"}}, "", 0, ""},
		{"empty-response", nil, "", 0, ""},
		{"model-failure-with-response", []llmItem{llmMessage("a", "not used")}, "oracle provider failure", 0, ""},
		{"model-failure-nil-response", nil, "oracle provider failure", 0, ""},
		{"parent-deadline", []llmItem{llmMessage("a", "brief")}, "", 17 * time.Second, "deadline"},
		{"cancelled", nil, "", 0, "cancelled"},
		{"expired", nil, "", 0, "expired"},
		{"nil-model", nil, "", 0, ""},
		{"empty-transcript", nil, "", 0, ""},
		{"nil-transcript", nil, "", 0, ""},
		{"ignored-transcript", nil, "", 0, ""},
		{"large-transcript", []llmItem{llmMessage("a", "brief")}, "", 0, ""},
	}
	out := []map[string]any{}
	for _, c := range cases {
		ctx := context.Background()
		cancel := func() {}
		switch c.parent {
		case "deadline":
			ctx, cancel = context.WithTimeout(ctx, 5*time.Second)
		case "cancelled":
			ctx, cancel = context.WithCancel(ctx)
			cancel()
		case "expired":
			ctx, cancel = context.WithDeadline(ctx, time.Now().Add(-time.Second))
		}
		input := removed
		if c.name == "nil-transcript" {
			input = nil
		}
		if c.name == "large-transcript" {
			input = []llmItem{}
			for i := 0; i < 250; i++ {
				s := llmMessage("a", "")
				s.Text = llmText{Prefix: fmt.Sprintf("segment-%03d:", i), Repeat: "界", Count: 400}
				input = append(input, s)
			}
		}
		if c.name == "empty-transcript" {
			input = []llmItem{llmMessage("", " \u2003")}
		}
		if c.name == "ignored-transcript" {
			input = []llmItem{{Kind: "compaction", Text: llmText{Prefix: "opaque"}}}
		}
		m := &llmOracleModel{t: t, response: &ModelResponse{Items: llmItems(c.items), Usage: usage}, timeout: c.timeout, parent: ctx, calls: []map[string]any{}, contextError: c.parent == "cancelled" || c.parent == "expired"}
		if c.err != "" {
			m.err = errors.New(c.err)
		}
		if c.name == "model-failure-nil-response" {
			m.response = nil
		}
		var model Model = m
		if c.name == "nil-model" {
			model = nil
		}
		body, gotUsage, err := summarizeRemovedItemsWithModel(ctx, model, "oracle/model", llmItems(input), c.timeout)
		cancel()
		errText := ""
		if err != nil {
			errText = err.Error()
		}
		out = append(out, map[string]any{"name": c.name, "removed": input, "responseItems": c.items, "responseNil": m.response == nil, "responseUsage": usage, "modelError": c.err, "nilModel": model == nil, "parentContext": c.parent, "timeoutNanos": int64(c.timeout), "requests": m.calls, "body": body, "usage": gotUsage, "error": errText})
	}
	return out
}

func llmPlanObservations(t *testing.T) []map[string]any {
	user := llmMessage("", "Original task: fix the composer")
	source := []llmItem{user}
	for i := 0; i < 8; i++ {
		source = append(source, llmItem{Kind: "reasoning", Text: llmText{Repeat: "design thought ", Count: 40}}, llmItem{Kind: "toolCall", ID: fmt.Sprint(i), Name: "read_file", Input: llmText{Prefix: `{"path":"a.go"}`}}, llmItem{Kind: "toolOutput", ID: fmt.Sprint(i), Text: llmText{Repeat: "file content ", Count: 60}})
	}
	source = append(source, llmMessage("assistant", "TAIL: edit a.go next"))
	cfg := CompactionConfig{Enabled: true, TriggerTokens: 100, TargetTokens: 80, PreserveRecentItems: 3, PreserveInitialUserMessages: 1, SummaryBulletLimit: 4}
	plan, _, ok, reason := planRunItemsCompaction(llmItems(source), cfg)
	if !ok {
		t.Fatal(reason)
	}
	out := []map[string]any{}
	for _, name := range []string{"planner-success", "planner-empty", "planner-failure", "planner-nil-model", "planner-nonshrinking", "equal-tokens", "one-token-smaller", "one-token-larger", "deferred-summary"} {
		p := plan
		specs := source
		planner := true
		body := llmText{Prefix: " Findings: a.go needs a status strip.\nNext: edit and test. "}
		if name == "planner-empty" {
			body = llmText{Prefix: " [COMPACTED HISTORY SUMMARY] "}
		}
		if name == "planner-nonshrinking" {
			body = llmText{Repeat: "verbose ", Count: 5000}
		}
		if name == "equal-tokens" || name == "one-token-smaller" || name == "one-token-larger" {
			planner = false
			body = llmText{Prefix: "brief"}
			specs = []llmItem{llmMessage("", "x")}
			p = CompactionPlan{Source: llmItems(specs), Removed: llmItems(specs), Protected: map[int]struct{}{}}
			n := (estimateRunItemsTokens(p.RebuildWithSummary(body.expand())) - 9) * 4
			if name == "one-token-smaller" {
				n += 4
			}
			if name == "one-token-larger" {
				n -= 4
			}
			specs[0].Text = llmText{Repeat: "x", Count: n}
			p.Source = llmItems(specs)
			p.Removed = llmItems(specs)
			p.Items = p.RebuildWithSummary("fallback")
			p.After = estimateRunItemsTokens(p.Items)
		}
		if name == "deferred-summary" {
			planner = false
			specs = []llmItem{llmMessage("assistant", strings.Repeat("old ", 1000)), user, llmMessage("assistant", "recent")}
			p = CompactionPlan{Source: llmItems(specs), Removed: llmItems(specs[:1]), Protected: map[int]struct{}{1: {}, 2: {}}}
			p.Items = p.RebuildWithSummary("fallback")
			p.After = estimateRunItemsTokens(p.Items)
		}
		usage := Usage{Requests: 2, InputTokens: 601, OutputTokens: 47, CacheReadTokens: 31, CacheCreateTokens: 11}
		m := &llmOracleModel{t: t, response: &ModelResponse{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: body.expand()}}}, Usage: usage}, parent: context.Background(), calls: []map[string]any{}}
		if name == "planner-failure" {
			m.err = errors.New("oracle provider failure")
		}
		var model Model = m
		if name == "planner-nil-model" {
			model = nil
		}
		beforeJSON, _ := json.Marshal(p)
		rebuilt, gotUsage, accepted := applyLLMSummaryToPlan(context.Background(), model, "oracle/model", p, 0)
		afterJSON, _ := json.Marshal(p)
		unchanged := string(beforeJSON) == string(afterJSON)
		if !unchanged {
			t.Fatal("apply mutated plan")
		}
		protected := []int{}
		for i := range p.Protected {
			protected = append(protected, i)
		}
		sort.Ints(protected)
		removed := []int{}
		removedItems := []RunItem{}
		for i := range p.Source {
			if _, keep := p.Protected[i]; !keep {
				removed = append(removed, i)
				removedItems = append(removedItems, p.Source[i])
			}
		}
		if !reflect.DeepEqual(removedItems, p.Removed) {
			t.Fatal("removed indices do not reconstruct the SDK plan")
		}
		probe := &llmOracleModel{t: t, response: m.response, err: m.err, parent: context.Background()}
		var candidateModel Model = probe
		if model == nil {
			candidateModel = nil
		}
		candidateBody, _, candidateErr := summarizeRemovedItemsWithModel(context.Background(), candidateModel, "oracle/model", p.Removed, 0)
		candidateError := ""
		if candidateErr != nil {
			candidateError = candidateErr.Error()
		}
		candidate := p.RebuildWithSummary(candidateBody)
		out = append(out, map[string]any{"name": name, "usesPlanner": planner, "config": cfg, "source": specs, "removedIndices": removed, "protectedIndices": protected, "deterministicItems": llmItemObservations(p.Items), "planAfter": p.After, "sourceTokens": estimateRunItemsTokens(p.Source), "responseBody": body, "responseUsage": usage, "modelError": m.err != nil, "nilModel": model == nil, "candidateBody": llmStringObservation(candidateBody), "candidateSummaryError": candidateError, "candidateTokens": estimateRunItemsTokens(candidate), "candidateItems": llmItemObservations(candidate), "accepted": accepted, "items": llmItemObservations(rebuilt), "usage": gotUsage, "requests": m.calls, "planUnchanged": unchanged})
	}
	return out
}

func TestNativeLLMSummaryReference(t *testing.T) {
	result := map[string]any{"schemaVersion": 1, "constants": map[string]any{"maxOutputTokens": llmSummaryMaxOutputTokens, "transcriptByteBudget": llmSummaryTranscriptCharBudget, "defaultTimeoutNanos": int64(llmSummaryCallTimeout), "instructions": llmSummaryInstructions}, "truncate": llmTruncateObservations(), "flatten": llmFlattenObservations(), "summaries": llmSummaryObservations(t), "plans": llmPlanObservations(t)}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("LLM_SUMMARY_OUTPUT"), append(data, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
}
