package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"time"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tracestore"
)

func writerFixture() {
	root, err := os.MkdirTemp("", "adk-writer-reference-")
	must(err)
	defer os.RemoveAll(root)
	store, err := tracestore.NewFilesystemTraceStore(root)
	must(err)
	defer store.Close()
	start := time.Date(2025, 1, 2, 3, 4, 5, 0, time.UTC)
	writer := tracestore.NewTraceWriter(store)
	must(writer.InitRun(tracestore.RunMetadata{RunID: "run", StartedAt: start}))
	request := &agent.LLMRequestSnapshot{AgentName: "agent", Model: "requested", Instructions: "system instructions"}
	response := &agent.LLMResponseSnapshot{Texts: []string{"answer"}, Usage: agent.Usage{InputTokens: 12, OutputTokens: 3}}
	requestBytes, err := json.Marshal(request)
	must(err)
	responseBytes, err := json.Marshal(response)
	must(err)
	data := []agent.SpanData{
		agent.GenerationSpanData{RequestedModel: "requested", ResolvedModel: "resolved", ModelProvider: "provider", ModelCanonical: "canonical", AttemptNumber: 2, Turn: 3, Scope: "root", TaskID: "task", Status: "completed", UsageAvailable: true, PromptTokens: 12, CompletionTokens: 3, CacheReadTokens: 4, CacheCreateTokens: 2, TotalTokens: 15, CostUSD: 0.5, CostKnown: true, LatencyMS: 1000, Success: true, ToolCount: 1, InputItemCount: 2, OutputItemCount: 1, InstructionsLength: 19, InputTokenEstimate: 8, RequestOverheadTokenEstimate: 4, TotalRequestTokenEstimate: 12, Request: request, Response: response},
		agent.FunctionSpanData{ToolName: "Read", IsError: true},
		agent.HandoffSpanData{FromAgent: "a", ToAgent: "b"},
		agent.GuardrailSpanData{GuardrailName: "guard", Triggered: true},
		agent.CompactionSpanData{TokensBefore: 100, TokensAfter: 50},
		agent.SessionSpanData{Model: "model", CostUSD: 0.5, NumTurns: 2, DurationMS: 900, InputTokens: 12, OutputTokens: 3, CacheReadInputTokens: 4, CacheCreationInputTokens: 2, StopReason: "completed"},
		agent.SubagentSpanData{TaskID: "child", Type: "researcher", Description: "task description", Model: "model", Status: "completed", CostUSD: 0.5, NumTurns: 2, TotalTokens: 15, InputTokens: 12, OutputTokens: 3, CacheReadTokens: 4, CacheCreateTokens: 2, ToolCount: 1, DurationMS: 900, StopReason: "completed", Isolation: "worktree", Prompt: "child prompt", ResultText: "child result", FilesRead: []string{"input.rs"}, FilesWritten: []string{"output.rs"}},
		agent.AgentSpanData{AgentName: "agent"},
		agent.RetrySpanData{ErrorCode: "rate_limit", Attempt: 2},
	}
	trace := &agent.Trace{ID: "trace", Name: "fixture", StartTime: start, EndTime: start.Add(time.Second)}
	writer.OnTraceStart(trace)
	for _, d := range data {
		span := &agent.Span{ID: "span", ParentID: "trace", Name: "operation", StartTime: start, EndTime: start.Add(time.Second), Data: d}
		writer.OnSpanStart(span)
		writer.OnSpanEnd(span)
	}
	writer.OnTraceEnd(trace)
	a := &agent.Agent{Name: "agent", Model: "model"}
	writer.OnAgentStart(nil, a)
	writer.OnLLMStart(nil, a)
	writer.OnLLMEnd(nil, a, &agent.ModelResponse{Items: []agent.RunItem{{Type: agent.RunItemMessage, Message: &agent.MessageOutput{Text: "answer"}}}, Usage: agent.Usage{InputTokens: 12, OutputTokens: 3}})
	tool := &agent.FunctionTool{ToolName: "Bash"}
	call := agent.ToolCallData{ID: "call", Name: "Bash", Input: json.RawMessage(`{"command":"pwd"}`)}
	writer.OnToolStart(&agent.RunContext{}, a, tool, call)
	writer.OnToolEnd(&agent.RunContext{}, a, tool, call, agent.ToolResult{Content: "tool output"})
	writer.OnHandoff(nil, a, &agent.Agent{Name: "specialist"})
	writer.OnAgentEnd(nil, a, nil)
	writer.RecordPhaseChange("verify")
	writer.RecordModeSwitch("code", "plan")
	writer.WriteResolvedInstructions(4, "other instructions")
	writer.WriteMetrics(map[string]any{"turns": 2})
	writer.FinalizeRun("completed")
	path, err := store.RunDir("run")
	must(err)
	result := map[string]any{"request_json": string(requestBytes), "response_json": string(responseBytes)}
	for _, category := range []string{"spans", "llm_calls", "tool_calls", "agent_transitions"} {
		bytes, err := os.ReadFile(filepath.Join(path, category+".jsonl"))
		must(err)
		var records []any
		for _, line := range strings.Split(strings.TrimSpace(string(bytes)), "\n") {
			var record map[string]any
			must(json.Unmarshal([]byte(line), &record))
			delete(record, "timestamp")
			delete(record, "start_time")
			delete(record, "end_time")
			if category == "tool_calls" {
				delete(record, "duration_ms")
			}
			records = append(records, record)
		}
		result[category] = records
	}
	for _, name := range []string{"resolved_instructions/turn_003_attempt_002.txt", "resolved_instructions/turn_004.txt", "trace_health.json", "metrics.json"} {
		bytes, err := os.ReadFile(filepath.Join(path, name))
		must(err)
		var value any
		must(json.Unmarshal(bytes, &value))
		result[name] = value
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(result))
}
