package agentsdk

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"io"
	"log"
	"math"
	"os"
	"strconv"
	"strings"
	"testing"
)

func TestNativeContentLoggerReference(t *testing.T) {
	oldWriter, oldFlags, oldPrefix := log.Writer(), log.Flags(), log.Prefix()
	defer func() { log.SetOutput(oldWriter); log.SetFlags(oldFlags); log.SetPrefix(oldPrefix) }()
	log.SetFlags(0)
	log.SetPrefix("")
	var output bytes.Buffer
	log.SetOutput(&output)
	logger := NewAgentLogger(LogLevelNormal)
	events := []ContentEvent{}
	for _, kind := range []string{"assistant_text", "assistant_thinking", "tool_start", "tool_end", "phase_transition", "subagent_status", "session_end", "llm_attempt", "system_init", "", "unknown\n"} {
		events = append(events, ContentEvent{Type: kind}, ContentEvent{Type: kind, Message: "hello\n\"world\"\x00\a\b\f\r\t\v\u0085\u00a0\u00ad\u200b\u2028😀", Output: "provider error", Tool: "shell\n", ToolUseID: "id", AgentName: "agent", IsError: true, Phase: "phase", TaskID: "task", Status: "failed", SubagentType: "researcher", SubagentModel: "child", AttemptStatus: "failed", Model: "model", Provider: "provider", FailureKind: "timeout", AttemptLatencyMs: -1})
	}
	for _, n := range []int{197, 198, 199, 200, 201, 997, 998, 999, 1000, 1001} {
		message := strings.Repeat("x", n) + "😀suffix"
		events = append(events, ContentEvent{Type: "assistant_text", Message: message}, ContentEvent{Type: "llm_attempt", Output: message})
	}
	type observation struct {
		Event ContentEvent `json:"event"`
		Log   string       `json:"log"`
	}
	cases := []observation{}
	for _, event := range events {
		output.Reset()
		logger.Event(event)
		cases = append(cases, observation{event, output.String()})
	}
	var all strings.Builder
	ranges := [][2]int{}
	start := -1
	for r := 0; r <= 0x110000; r++ {
		printable := r < 0x110000 && strconv.IsPrint(rune(r))
		if printable && start < 0 {
			start = r
		}
		if !printable && start >= 0 {
			ranges = append(ranges, [2]int{start, r - 1})
			start = -1
		}
		if r < 0x110000 && !(r >= 0xd800 && r <= 0xdfff) {
			all.WriteRune(rune(r))
		}
	}
	output.Reset()
	logger.Event(ContentEvent{Type: "phase_transition", Message: all.String()})
	digest := sha256.Sum256(output.Bytes())
	result := map[string]any{"cases": cases, "printableRanges": ranges, "allUnicodeLogSHA256": hex.EncodeToString(digest[:]), "allUnicodeLogBytes": output.Len()}
	output.Reset()
	parent := NewEventStream(io.Discard)
	parent.SetLogger(logger)
	child := NewChildEventStream(parent, "child")
	parent.SetLogger(nil)
	parent.Emit(ContentEvent{Type: "not_logged"})
	child.Emit(ContentEvent{Type: "subagent_status", SubagentType: "researcher", Message: "first"})
	NewChildEventStream(parent, "later").Emit(ContentEvent{Type: "not_logged"})
	child.SetLogger(nil)
	child.Emit(ContentEvent{Type: "not_logged"})
	parent.SetLogger(logger)
	parent.Emit(ContentEvent{Type: "subagent_status", TaskID: "child", Message: "inherited name"})
	parent.Emit(ContentEvent{Type: "phase_transition", Message: "not serializable", CostUsd: math.NaN()})
	result["streamLog"] = output.String()
	encoded, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("METADATA_OUTPUT"), encoded, 0600); err != nil {
		t.Fatal(err)
	}
}
