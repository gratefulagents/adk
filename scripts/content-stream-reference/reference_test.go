// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"math"
	"os"
	"strings"
	"testing"
	"time"
	"unicode"
)

type nativeFailedContentWriter struct{ attempts int }

func (w *nativeFailedContentWriter) Write(p []byte) (int, error) {
	w.attempts++
	return 0, io.ErrClosedPipe
}

func TestNativeContentStreamReference(t *testing.T) {
	start := time.Now()
	record := func(event ContentEvent) map[string]any {
		automatic := !event.Timestamp.Before(start)
		if automatic {
			_, offset := event.Timestamp.Zone()
			if offset != 0 {
				t.Fatal("expected UTC reference profile")
			}
			if event.Timestamp.After(time.Now()) {
				t.Fatal("future automatic timestamp")
			}
			event.Timestamp = time.Time{}
		}
		nonfinite := math.IsNaN(event.CostUsd)
		if nonfinite {
			event.CostUsd = 0
		}
		encoded, err := json.Marshal(event)
		if err != nil {
			t.Fatal(err)
		}
		return map[string]any{"automatic": automatic, "nonfinite": nonfinite, "json": string(encoded)}
	}
	drain := func(ch <-chan ContentEvent) map[string]any {
		events := []map[string]any{}
		closed := false
	loop:
		for {
			select {
			case event, ok := <-ch:
				if !ok {
					closed = true
					break loop
				}
				events = append(events, record(event))
			default:
				break loop
			}
		}
		return map[string]any{"events": events, "closed": closed}
	}
	wire := func(buf *bytes.Buffer) []map[string]any {
		events := []map[string]any{}
		for _, line := range strings.Split(strings.TrimSpace(buf.String()), "\n") {
			if line == "" {
				continue
			}
			var event ContentEvent
			if err := json.Unmarshal([]byte(line), &event); err != nil {
				t.Fatal(err)
			}
			events = append(events, record(event))
		}
		return events
	}
	var output bytes.Buffer
	root := NewEventStream(&output)
	root.SetSession(7)
	root.SetPhase("planning")
	root.SetStep("exploring")
	child := NewChildEventStream(root, "task")
	root.SetSession(8)
	root.SetPhase("implementation")
	root.SetStep("editing")
	sub, unsubscribe := child.Subscribe(16)
	root.Emit(ContentEvent{Type: "root", Message: "message"})
	child.Emit(ContentEvent{Type: "child"})
	fixed := time.Date(2020, 2, 3, 4, 5, 6, 123400000, time.FixedZone("east", 19800))
	child.Emit(ContentEvent{Type: "explicit", Session: 9, Phase: "phase", Step: "step", TaskID: "override", Timestamp: fixed})
	child.Emit(ContentEvent{Type: "zero_offset", Timestamp: time.Date(1, 1, 1, 1, 0, 0, 0, time.FixedZone("east", 3600))})
	child.Emit(ContentEvent{Type: "zero_large_offset", Timestamp: time.Date(1, 1, 2, 1, 0, 0, 0, time.FixedZone("east", 90000))})
	root.SetSession(0)
	root.SetPhase("")
	root.SetStep("")
	root.Emit(ContentEvent{Type: "cleared"})
	root.SetSession(-2147483648)
	root.SetPhase("界\nphase")
	root.SetStep(" \tstep")
	root.Emit(ContentEvent{Type: "minimum"})
	root.SetSession(2147483647)
	root.Emit(ContentEvent{Type: "maximum"})
	metadata := map[string]any{"subscriber": drain(sub), "wire": wire(&output)}
	unsubscribe()
	unsubscribe()
	root.Emit(ContentEvent{Type: "after_close"})
	metadata["closed"] = drain(sub)

	output.Reset()
	root = NewEventStream(&output)
	child = NewChildEventStream(root, "task")
	for _, event := range []ContentEvent{
		{Type: "subagent_status", TaskID: "task", SubagentType: "worker", AgentName: "explicit"},
		{Type: "subagent_status"},
		{Type: "subagent_status", AgentName: "renamed"},
		{Type: "subagent_status"},
		{Type: "other"},
		{Type: "subagent_status", TaskID: "unknown"},
	} {
		child.Emit(event)
	}
	annotation := wire(&output)

	drops := []map[string]any{}
	for _, capacity := range []int{1, 2, 0, -1} {
		output.Reset()
		root = NewEventStream(&output)
		sub, unsubscribe = root.Subscribe(capacity)
		count := 4
		if capacity <= 0 {
			count = 66
		}
		for i := 0; i < count; i++ {
			root.Emit(ContentEvent{Type: "tick", Message: fmt.Sprint(i)})
		}
		first := drain(sub)
		root.Emit(ContentEvent{Type: "later"})
		second := drain(sub)
		root.Emit(ContentEvent{Type: "last"})
		third := drain(sub)
		unsubscribe()
		drops = append(drops, map[string]any{"capacity": capacity, "first": first, "second": second, "third": third, "closed": drain(sub)})
	}

	failed := &nativeFailedContentWriter{}
	root = NewEventStream(failed)
	sub, unsubscribe = root.Subscribe(4)
	root.Emit(ContentEvent{Type: "writer_failure"})
	root.Emit(ContentEvent{Type: "marshal_failure", CostUsd: math.NaN()})
	failure := map[string]any{"attempts": failed.attempts, "subscriber": drain(sub)}
	unsubscribe()
	output.Reset()
	root = NewEventStream(&output)
	root.SetSession(-7)
	root.SetPhase("helpers")
	child = NewChildEventStream(root, "task")
	for _, text := range []string{"", "line\n\"<&>😀", strings.Repeat("😀", 17000)} {
		child.EmitText(text)
		child.EmitText(text, "agent")
		child.EmitThinking(text)
		child.EmitThinking(text, "agent")
		child.EmitThinkingDelta(text, "agent", "attempt")
		child.EmitCompaction(2147483648, -2147483649, text)
		child.EmitHookDecision("hook", "tool", "deny", text)
	}
	result := map[string]any{"metadata": metadata, "annotation": annotation, "drops": drops, "failure": failure, "helpers": wire(&output)}
	oldWriter, oldFlags, oldPrefix := log.Writer(), log.Flags(), log.Prefix()
	defer func() { log.SetOutput(oldWriter); log.SetFlags(oldFlags); log.SetPrefix(oldPrefix) }()
	log.SetFlags(0)
	log.SetPrefix("")
	sessions := []map[string]any{}
	for i := 0; i < 4; i++ {
		output.Reset()
		var logs bytes.Buffer
		log.SetOutput(&logs)
		opts := SessionEventStreamOptions{}
		if i > 0 {
			opts.SystemInit = &SessionSystemInit{}
		}
		if i > 1 {
			opts.Session, opts.Phase = -7, "plan"
			opts.EnableLogger = true
			opts.SystemInit = &SessionSystemInit{Model: "model", PermissionMode: "read-only", Cwd: "/repo/😀", MaxTurns: 2147483648, Tools: []string{"Read", "", "Read"}, MCPServers: []string{"docs"}}
		}
		if i == 3 {
			opts.Logger = NewAgentLogger(LogLevelDebug)
			opts.SystemInit.MaxTurns = -2147483649
		}
		root = NewSessionEventStream(&output, opts)
		sub, unsubscribe = root.Subscribe(4)
		root.EmitSessionEnd("completed", 0.125, true, -1, 9223372036854775807, 3, 4, -2147483649, -9, "done\n😀")
		root.EmitSessionEnd("", 0, false, 0, 0, 0, 0, 0, 0, "")
		sessions = append(sessions, map[string]any{"wire": wire(&output), "subscriber": drain(sub), "logs": logs.String()})
		unsubscribe()
	}
	result["sessions"] = sessions
	toolCases := []map[string]any{}
	texts := []string{"", " \n😀\r\n ", strings.Repeat("x", 32768), strings.Repeat("x", 32769)}
	for _, n := range []int{32762, 32763, 32764, 32765} {
		texts = append(texts, strings.Repeat("x", n)+"😀tail")
	}
	texts = append(texts, strings.Repeat("😀", 20000))
	compactRecords := func(records []map[string]any) []map[string]any {
		for _, record := range records {
			text := record["json"].(string)
			record["bytes"] = len(text)
			record["sha256"] = fmt.Sprintf("%x", sha256.Sum256([]byte(text)))
			delete(record, "json")
		}
		return records
	}
	for i, text := range texts {
		output.Reset()
		root = NewEventStream(&output)
		root.SetSession(5)
		root.SetPhase("tools")
		child = NewChildEventStream(root, "task")
		sub, unsubscribe = child.Subscribe(4)
		name, call, parent, agent, summary := "tool\n", "call", "parent", "agent😀", "summary\n"+text
		if i == 0 {
			name, call, parent, agent, summary = "", "", "", "", ""
		}
		child.EmitToolStart(name, call, parent, summary, agent, text)
		child.EmitToolEnd(name, call, parent, false, agent, text, -9223372036854775808)
		child.EmitToolEnd(name, call, parent, true, agent, text, 9223372036854775807)
		delivered := drain(sub)
		delivered["events"] = compactRecords(delivered["events"].([]map[string]any))
		toolCases = append(toolCases, map[string]any{"wire": compactRecords(wire(&output)), "subscriber": delivered})
		unsubscribe()
	}
	result["toolLimit"] = DefaultMaxToolResultBytes
	result["toolCases"] = toolCases
	subagentCases := []map[string]any{}
	var spaces strings.Builder
	for r := rune(0); r <= 0x10ffff; r++ {
		if unicode.IsSpace(r) {
			spaces.WriteRune(r)
		}
	}
	result["subagentWhitespace"] = spaces.String()
	subagentTexts := []string{"", spaces.String(), " \n😀\rinterior\t\ntext\x1c\x1f ", strings.Repeat("a", 4096), strings.Repeat("a", 4097), strings.Repeat("😀", 4096), strings.Repeat("😀", 4097), strings.Repeat("a", 4092) + "e\u0301tail", strings.Repeat(" \n😀\n", 2000), "\ufeff prefix\u200b\n😀 \ufeff"}
	capture := func() map[string]any {
		delivered := drain(sub)
		delivered["events"] = compactRecords(delivered["events"].([]map[string]any))
		return map[string]any{"wire": compactRecords(wire(&output)), "subscriber": delivered}
	}
	for _, text := range subagentTexts {
		output.Reset()
		root = NewEventStream(&output)
		root.SetSession(3)
		root.SetPhase("workers")
		child = NewChildEventStream(root, "task")
		child.SetStep("work")
		sub, unsubscribe = child.Subscribe(8)
		child.EmitSubagentCompleted("", "cold", "", 0, 0, 0, 0, false, 0, "", "")
		child.EmitSubagentStarted("", "launch", text, "worker😀", "model", text)
		child.EmitSubagentCompletedWithUsage("", "failed", text, -2147483648, -9223372036854775808, 9223372036854775807, -0.125, true, 2147483647, Usage{InputTokens: 99, CacheReadTokens: -7, CacheCreateTokens: 9223372036854775807}, "stop\n", text)
		child.EmitSubagentCompleted("", "other", text, 2147483647, 9223372036854775807, -9223372036854775808, 0.375, false, -2147483648, "other", text)
		subagentCases = append(subagentCases, capture())
		unsubscribe()
	}
	result["subagentCases"] = subagentCases
	statusCases := []map[string]any{}
	statuses := []string{"", "pending", "waiting", "running", "reconciling", "completed", "failed", "cancelled", "canceled", "Completed", "unknown"}
	durations := []int64{-1, -999999, -1000001, -9223372036854775808, 9223372036854775807, 0, 1000001}
	for i, status := range statuses {
		output.Reset()
		root = NewEventStream(&output)
		root.SetSession(3)
		root.SetPhase("workers")
		child = NewChildEventStream(root, "task")
		sub, unsubscribe = child.Subscribe(8)
		child.EmitSubagentStarted("", "", "", "prior", "", "")
		task := SubAgentTask{Status: SubAgentTaskStatus(status), Message: " \n" + strings.Repeat("😀", 4097) + "\n ", Duration: time.Duration(durations[i%len(durations)]), Result: " result\n" + strings.Repeat("r", 4097), Error: "must not appear", ToolCount: -7, Tokens: -11, DependsOn: []string{"first", "", "first"}, WaitingOn: []string{"first"}, MessagesReceived: -3, LastParentMessage: strings.Repeat("parent\n", 1000), CurrentStep: "step", LastTool: "tool", FilesWritten: 9223372036854775807}
		if i%2 != 0 {
			task.AgentName = "agent"
		}
		if status == "failed" {
			task.Result = ""
		}
		if i == 0 {
			task.DependsOn = nil
			task.WaitingOn = []string{}
		}
		child.EmitSubagentTaskStatus(task, "")
		child.EmitSubagentTaskStatus(task, " \n")
		statusCases = append(statusCases, capture())
		unsubscribe()
	}
	result["subagentStatusCases"] = statusCases
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
