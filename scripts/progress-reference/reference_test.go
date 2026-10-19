// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/json"
	"fmt"
	"os"
	"testing"
	"time"
)

// The fixture supplies stimuli only. All observations come from pinned SDK methods.
type progressOp struct {
	Op     string   `json:"op"`
	Target string   `json:"target"`
	Args   []string `json:"args"`
	N      []int64  `json:"n"`
	Cost   float64  `json:"cost"`
}

func (o progressOp) s(i int) string {
	if i < len(o.Args) {
		return o.Args[i]
	}
	return ""
}
func (o progressOp) n(i int) int64 {
	if i < len(o.N) {
		return o.N[i]
	}
	return 0
}
func (o progressOp) usage() Usage {
	return Usage{InputTokens: o.n(0), OutputTokens: o.n(1), CacheReadTokens: o.n(2), CacheCreateTokens: o.n(3)}
}

type progressCase struct {
	Name string       `json:"name"`
	Ops  []progressOp `json:"ops"`
}
type progressProbe struct {
	NoOpTracingProcessor
	IDs   map[string]string
	Calls []any
}

func (p *progressProbe) capture(phase string, s *Span) {
	id, ok := p.IDs[s.ID]
	if !ok {
		id = fmt.Sprintf("span-%d", len(p.IDs)+1)
		p.IDs[s.ID] = id
	}
	var data any
	switch d := s.Data.(type) {
	case SessionSpanData:
		data = map[string]any{"kind": "session", "model": d.Model, "cost": d.CostUSD, "turns": d.NumTurns, "duration_ms": d.DurationMS, "input": d.InputTokens, "output": d.OutputTokens, "cache_read": d.CacheReadInputTokens, "cache_create": d.CacheCreationInputTokens, "stop": d.StopReason}
	case SubagentSpanData:
		// Only measured elapsed time is normalized; explicit session/retry durations remain intact.
		data = map[string]any{"kind": "subagent", "task": d.TaskID, "type": d.Type, "description": d.Description, "model": d.Model, "status": d.Status, "cost": d.CostUSD, "turns": d.NumTurns, "total": d.TotalTokens, "input": d.InputTokens, "output": d.OutputTokens, "cache_read": d.CacheReadTokens, "cache_create": d.CacheCreateTokens, "tools": d.ToolCount, "duration_ms": "<elapsed>", "stop": d.StopReason, "isolation": d.Isolation, "prompt": d.Prompt, "result": d.ResultText, "files_read": append([]string{}, d.FilesRead...), "files_written": append([]string{}, d.FilesWritten...)}
	case RetrySpanData:
		data = map[string]any{"kind": "retry", "error": d.ErrorCode, "after_ms": d.RetryAfterMS, "attempt": d.Attempt, "max": d.MaxRetries}
	case CompactionSpanData:
		data = map[string]any{"kind": "compaction", "before": d.TokensBefore, "after": d.TokensAfter}
	default:
		panic(fmt.Sprintf("unhandled span %T", s.Data))
	}
	p.Calls = append(p.Calls, map[string]any{"phase": phase, "id": id, "parent": s.ParentID, "name": s.Name, "finished": !s.EndTime.IsZero(), "data": data})
}
func (p *progressProbe) OnSpanStart(s *Span) { p.capture("start", s) }
func (p *progressProbe) OnSpanEnd(s *Span)   { p.capture("end", s) }
func (p *progressProbe) OnTraceStart(*Trace) { p.Calls = append(p.Calls, "trace_start") }
func (p *progressProbe) OnTraceEnd(*Trace)   { p.Calls = append(p.Calls, "trace_end") }
func (p *progressProbe) Flush()              { p.Calls = append(p.Calls, "flush") }
func normalizeProgress(v any) any {
	b, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	var out any
	if err = json.Unmarshal(b, &out); err != nil {
		panic(err)
	}
	var walk func(any)
	walk = func(x any) {
		switch z := x.(type) {
		case map[string]any:
			for k, v := range z {
				if k == "timestamp" {
					z[k] = "<time>"
				} else {
					walk(v)
				}
			}
		case []any:
			for _, v := range z {
				walk(v)
			}
		}
	}
	walk(out)
	return out
}
func TestNativeProgressReference(t *testing.T) {
	raw, err := os.ReadFile(os.Getenv("PROGRESS_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []progressCase
	if err = json.Unmarshal(raw, &cases); err != nil {
		t.Fatal(err)
	}
	results := map[string]any{}
	for _, c := range cases {
		t.Run(c.Name, func(t *testing.T) {
			probe := &progressProbe{IDs: map[string]string{}, Calls: []any{}}
			root := NewProgressTracker(WithTracingProcessor(probe), WithMaxToolResultBytes(7))
			root.SetRootSpanID("root-A")
			trackers := map[string]*ProgressTracker{"root": root}
			rows := []any{}
			for _, o := range c.Ops {
				target := o.Target
				if target == "" {
					target = "root"
				}
				tr := trackers[target]
				if tr == nil {
					t.Fatalf("unknown target %q", target)
				}
				var extra any
				switch o.Op {
				case "snapshot":
				case "session":
					tr.SetSession(int32(o.n(0)), o.s(0))
				case "step":
					tr.SetStep(o.s(0))
				case "init":
					tr.RecordSystemInit("model", "cwd", nil, 4, nil, "")
				case "text":
					tr.RecordAssistantText(o.s(0))
				case "thinking":
					tr.RecordAssistantThinking(o.s(0))
				case "tool":
					tr.RecordToolUse(o.s(0), o.s(1), "call-1", 1, o.s(2), "agent", "")
				case "tool_result":
					tr.RecordToolResult("call-1", o.s(0), o.s(1), o.n(0) != 0, 42, "agent", "")
				case "lifecycle":
					tr.RecordLifecycleEvent(o.s(0), o.s(1))
				case "result":
					tr.WriteResult(o.s(0), o.s(1), o.s(2), o.s(3))
				case "hook":
					tr.RecordHookDecision(HookEvent{ToolName: "Write", Type: "pre"}, HookResult{Decision: "deny", Reason: "policy"})
				case "usage":
					tr.RecordLLMUsage(o.s(0), o.Cost, o.usage())
				case "complete":
					tr.RecordSessionComplete(o.s(0), o.Cost, int(o.n(4)), o.usage(), time.Duration(o.n(5))*time.Millisecond, o.s(1))
				case "child":
					trackers[o.s(0)] = NewChildTracker(tr, o.s(0))
				case "root_id":
					tr.SetRootSpanID(o.s(0))
				case "start":
					tr.RecordSubagentStarted(o.s(0), "call-1", o.s(1), o.s(2), o.s(3), o.s(4), o.s(5))
				case "progress":
					tr.RecordSubagentProgress(o.s(0), int32(o.n(0)), o.n(1), o.n(2), o.s(1))
				case "finish":
					tr.RecordSubagentCompleted(o.s(0), o.s(1), o.s(2), o.Cost, int(o.n(4)), o.usage(), o.s(3), []string{"src/read.rs"}, []string{"src/write.rs"})
				case "retry":
					tr.RecordAPIRetry(o.s(0), o.n(0), int32(o.n(1)), int32(o.n(2)))
				case "compact":
					tr.RecordCompactBoundary(int(o.n(0)), int(o.n(1)), o.s(0))
				case "drain":
					extra = tr.DrainPendingEvents()
				default:
					t.Fatalf("unknown op %s", o.Op)
				}
				states := map[string]any{}
				for name, tr := range trackers {
					modelWire, err := json.Marshal(tr.Snapshot().ModelUsage)
					if err != nil {
						t.Fatal(err)
					}
					states[name] = map[string]any{"snapshot": tr.Snapshot(), "model_usage_wire": string(modelWire), "text": tr.LastAssistantText(), "thinking": tr.LastAssistantThinking(), "session_tools": tr.sessionToolCallCount, "session_cost": tr.sessionCostUsd, "task": tr.taskID, "root": tr.rootSpanID, "max_bytes": tr.maxToolResultBytes, "agents": tr.taskAgents}
				}
				// Freeze mutable SDK maps and the callback list at each operation boundary.
				rows = append(rows, normalizeProgress(map[string]any{"states": states, "callbacks": probe.Calls, "extra": extra}))
			}
			results[c.Name] = rows
		})
	}
	b, err := json.MarshalIndent(map[string]any{"cases": cases, "observations": results}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("PROGRESS_OUTPUT"), append(b, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
}
