// GPL-3.0-only. Overlay into pinned internal/agent; never reimplements the planner.
package agent

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

type compactionReplayContent struct {
	Type   string `json:"type"`
	Text   string `json:"text"`
	Repeat int    `json:"repeat,omitempty"`
}
type compactionReplayMessage struct {
	Role    string                    `json:"role"`
	Content []compactionReplayContent `json:"content"`
}
type compactionReplayCall struct {
	ID        string          `json:"id"`
	Name      string          `json:"name"`
	Arguments json.RawMessage `json:"arguments"`
}
type compactionReplayOutput struct {
	Content     []compactionReplayContent `json:"content"`
	IsError     bool                      `json:"is_error"`
	ShouldPause bool                      `json:"should_pause"`
}
type compactionReplayItem struct {
	Type    string                   `json:"type"`
	Phase   string                   `json:"phase,omitempty"`
	Agent   *string                  `json:"agent,omitempty"`
	Message *compactionReplayMessage `json:"message,omitempty"`
	Call    *compactionReplayCall    `json:"call,omitempty"`
	CallID  string                   `json:"call_id,omitempty"`
	Output  *compactionReplayOutput  `json:"output,omitempty"`
}
type compactionReplayCase struct {
	Name     string                 `json:"name"`
	Policy   map[string]int         `json:"policy"`
	Disabled bool                   `json:"disabled"`
	Overhead int                    `json:"overhead"`
	History  []compactionReplayItem `json:"history"`
}

func compactionReplayText(parts []compactionReplayContent) string {
	out := []string{}
	for _, p := range parts {
		n := p.Repeat
		if n == 0 {
			n = 1
		}
		out = append(out, strings.Repeat(p.Text, n))
	}
	return strings.Join(out, "\n")
}
func compactionReplayConvert(items []compactionReplayItem) []RunItem {
	out := []RunItem{}
	for _, v := range items {
		switch v.Type {
		case "message":
			i := RunItem{Type: RunItemMessage, Message: &MessageOutput{Text: compactionReplayText(v.Message.Content)}}
			if v.Message.Role == "assistant" {
				i.Agent = &Agent{Name: "assistant"}
				if strings.HasPrefix(strings.TrimSpace(i.Message.Text), "[COMPACTED HISTORY SUMMARY]") {
					i.Agent.Name = "context-summary"
				}
			}
			out = append(out, i)
		case "tool_call", "approval":
			var args any
			if err := json.Unmarshal(v.Call.Arguments, &args); err != nil {
				panic(err)
			}
			canonical, err := json.Marshal(args)
			if err != nil {
				panic(err)
			}
			if v.Type == "approval" {
				if v.Phase != "pending" && v.Phase != "approved" && v.Phase != "denied" {
					panic("invalid explicit phase")
				}
				marker := RunItem{Type: RunItemToolApproval, ToolApproval: &ToolApprovalData{CallID: v.Call.ID, ToolName: v.Call.Name, Input: canonical, Approved: v.Phase == "approved"}}
				if v.Agent != nil {
					marker.Agent = &Agent{Name: *v.Agent}
				}
				out = append(out, marker)
			} else {
				out = append(out, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: v.Call.ID, Name: v.Call.Name, Input: canonical}})
			}
		case "tool_result":
			out = append(out, RunItem{Type: RunItemToolOutput, ToolOutput: &ToolOutputData{CallID: v.CallID, Content: compactionReplayText(v.Output.Content), IsError: v.Output.IsError}})
		default:
			panic("unsupported item: " + v.Type)
		}
	}
	return out
}

// Go's Approved=false does not distinguish pending and denied; preserve explicit fixture phases.
func compactionReplayApprovalKey(item RunItem) string {
	var agent *string
	if item.Agent != nil {
		agent = &item.Agent.Name
	}
	key, err := json.Marshal([]any{item.ToolApproval, agent})
	if err != nil {
		panic(err)
	}
	return string(key)
}
func compactionReplayNormalize(items []RunItem, phases map[string]string) []compactionReplayItem {
	out := []compactionReplayItem{}
	text := func(s string) []compactionReplayContent { return []compactionReplayContent{{Type: "text", Text: s}} }
	for _, v := range items {
		switch v.Type {
		case RunItemMessage:
			role := "user"
			if v.Agent != nil {
				role = "assistant"
			}
			out = append(out, compactionReplayItem{Type: "message", Message: &compactionReplayMessage{Role: role, Content: text(v.Message.Text)}})
		case RunItemToolCall:
			out = append(out, compactionReplayItem{Type: "tool_call", Call: &compactionReplayCall{ID: v.ToolCall.ID, Name: v.ToolCall.Name, Arguments: v.ToolCall.Input}})
		case RunItemToolApproval:
			phase, ok := phases[compactionReplayApprovalKey(v)]
			if !ok {
				panic("approval marker has no explicit phase")
			}
			var agent *string
			if v.Agent != nil {
				agent = &v.Agent.Name
			}
			out = append(out, compactionReplayItem{Type: "approval", Phase: phase, Agent: agent, Call: &compactionReplayCall{ID: v.ToolApproval.CallID, Name: v.ToolApproval.ToolName, Arguments: v.ToolApproval.Input}})
		case RunItemToolOutput:
			out = append(out, compactionReplayItem{Type: "tool_result", CallID: v.ToolOutput.CallID, Output: &compactionReplayOutput{Content: text(v.ToolOutput.Content), IsError: v.ToolOutput.IsError}})
		default:
			panic("unsupported normalized item")
		}
	}
	return out
}
func compactionReplayPolicy(c compactionReplayCase) CompactionConfig {
	cfg := DefaultCompactionConfig()
	cfg.Enabled = !c.Disabled
	cfg.UseLLMSummary = false
	for key, v := range c.Policy {
		switch key {
		case "trigger_tokens":
			cfg.TriggerTokens = v
		case "target_tokens":
			cfg.TargetTokens = v
		case "preserve_recent_items":
			cfg.PreserveRecentItems = v
		case "preserve_initial_user_messages":
			cfg.PreserveInitialUserMessages = v
		case "summary_bullet_limit":
			cfg.SummaryBulletLimit = v
		default:
			panic("unknown policy key: " + key)
		}
	}
	return cfg
}
func TestExportCompactionReference(t *testing.T) {
	data, err := os.ReadFile(os.Getenv("COMPACTION_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []compactionReplayCase
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	results := map[string]any{}
	for _, c := range cases {
		items := compactionReplayConvert(c.History)
		phases := map[string]string{}
		for index, item := range items {
			if item.Type != RunItemToolApproval {
				continue
			}
			key := compactionReplayApprovalKey(item)
			phase := c.History[index].Phase
			if previous, ok := phases[key]; ok && previous != phase {
				t.Fatal("ambiguous fixture approval phase")
			}
			phases[key] = phase
		}
		cfg := compactionReplayPolicy(c)
		history, before, after, changed, reason := MaybeCompactRunItemsForRequest(items, cfg, c.Overhead)
		finalHistory := history
		if changed {
			finalHistory, _ = applyCompactionCarryForward(context.Background(), history, items, RunConfig{}, cfg, c.Overhead)
		}
		results[c.Name] = map[string]any{"history": compactionReplayNormalize(history, phases), "final_history": compactionReplayNormalize(finalHistory, phases), "before_tokens": before, "after_tokens": after, "changed": changed, "reason": reason, "summary": ExtractCompactionSummary(history), "history_estimate": estimateRunItemsTokens(items), "normalized_policy": cfg.normalized()}
	}
	overhead := []any{}
	for _, settings := range []ModelSettings{{}, {MaxTokens: 100}, {MaxTokens: 100, ThinkingBudget: 200}, {MaxTokens: -1, ThinkingBudget: 20000}} {
		tools := []Tool{&FunctionTool{ToolName: "read", ToolDescription: "Read a file", Schema: json.RawMessage(`{"type":"object"}`)}}
		overhead = append(overhead, map[string]any{"max_tokens": settings.MaxTokens, "thinking_budget": settings.ThinkingBudget, "reserve": outputReserveTokens(settings), "overhead": estimateModelRequestOverheadTokens("instructions", tools, settings)})
	}
	stringsOut := []any{}
	for _, text := range []string{"", " \n\t", "abcd", "abc", "日本語🙂", "\u0085hello\u2003", "a\nb"} {
		stringsOut = append(stringsOut, map[string]any{"text": text, "tokens": estimateStringTokens(text)})
	}
	output := map[string]any{"cases": results, "default_policy": DefaultCompactionConfig(), "overhead": overhead, "strings": stringsOut, "cache_wire_key": promptCacheWireKey("namespace", "logical")}
	data, err = json.Marshal(output)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("COMPACTION_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
