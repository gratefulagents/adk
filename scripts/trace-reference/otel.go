package main

import (
	"context"
	"encoding/json"
	"io"
	"os"
	"time"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkotel "github.com/gratefulagents/sdk/pkg/agentsdk/otel"
	"golang.org/x/sys/unix"
)

func otelFixture() {
	output, err := os.CreateTemp("", "adk-otel-reference-")
	must(err)
	defer os.Remove(output.Name())
	defer output.Close()
	stdout := os.Stdout
	saved, err := unix.Dup(1)
	must(err)
	defer unix.Close(saved)
	must(unix.Dup2(int(output.Fd()), 1))
	processor, err := sdkotel.NewOTelTracingProcessorWithEndpoint(context.Background(), "fixture", "http:///")
	must(err)
	start := time.Date(2025, 1, 2, 3, 4, 5, 0, time.UTC)
	trace := &agent.Trace{ID: "trace", Name: "fixture", StartTime: start, EndTime: start.Add(time.Second)}
	processor.OnTraceStart(trace)
	data := []agent.SpanData{
		agent.AgentSpanData{AgentName: "agent", Instructions: "instruction"},
		agent.GenerationSpanData{RequestedModel: "requested", ResolvedModel: "resolved", AttemptNumber: 2, Turn: 3, Status: "completed", UsageAvailable: true, PromptTokens: 12, CompletionTokens: 3, TotalTokens: 15, Success: true, CostUSD: 0.5, InputTokensIncludeCache: true, InputTokensIncludeCacheKnown: true},
		agent.FunctionSpanData{ToolName: "Read", Input: "input", Output: "failure", IsError: true},
		agent.HandoffSpanData{FromAgent: "a", ToAgent: "b"},
		agent.GuardrailSpanData{GuardrailName: "guard", Triggered: true},
		agent.CompactionSpanData{TokensBefore: 100, TokensAfter: 50},
		agent.SessionSpanData{Model: "model", CostUSD: 0.5, NumTurns: 2, DurationMS: 900, InputTokens: 12, OutputTokens: 3, CacheReadInputTokens: 4, CacheCreationInputTokens: 2, StopReason: "completed"},
		agent.SubagentSpanData{TaskID: "child", Type: "researcher", Description: "task description", Model: "model", Status: "completed", CostUSD: 0.5, NumTurns: 2, TotalTokens: 15, InputTokens: 12, OutputTokens: 3, CacheReadTokens: 4, CacheCreateTokens: 2, ToolCount: 1, DurationMS: 900, StopReason: "completed", Isolation: "worktree"},
		agent.SubagentSpanData{Type: "executor", CacheReadTokens: 1200, CacheCreateTokens: 300},
		agent.RetrySpanData{ErrorCode: "rate_limit", Attempt: 2, RetryAfterMS: 500, MaxRetries: 3},
	}
	for _, data := range data {
		span := &agent.Span{ID: "span", ParentID: "trace", Name: "operation", StartTime: start, EndTime: start.Add(time.Second), Data: data}
		processor.OnSpanStart(span)
		processor.OnSpanEnd(span)
	}
	processor.OnTraceEnd(trace)
	must(processor.Shutdown(context.Background()))
	must(unix.Dup2(saved, 1))
	_, err = output.Seek(0, 0)
	must(err)
	decoder := json.NewDecoder(output)
	records := map[string]any{}
	for {
		var span struct {
			Name       string
			Attributes []struct {
				Key   string
				Value struct{ Value any }
			}
			Status any
		}
		err := decoder.Decode(&span)
		if err == io.EOF {
			break
		}
		must(err)
		attrs := map[string]any{}
		for _, attr := range span.Attributes {
			attrs[attr.Key] = attr.Value.Value
		}
		records[span.Name] = map[string]any{"attributes": attrs, "status": span.Status}
	}
	encoder := json.NewEncoder(stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(records))
}
