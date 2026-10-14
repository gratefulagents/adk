package agentsdk

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"log"
	"strings"
)

func agentLoggerObservations() []map[string]any {
	oldWriter := log.Writer()
	defer log.SetOutput(oldWriter)
	observations := []map[string]any{}
	for _, level := range []LogLevel{-2, 0, 1, 2} {
		var output bytes.Buffer
		log.SetOutput(&output)
		logger := NewAgentLogger(level)
		logger.Turn(-3, "agent\n😀", "model\"\\\x00")
		logger.TurnEnd(9223372036854775807)
		logger.Tools(nil, []string{}, ToolAccessLevel(""))
		logger.Tools([]string{"shell", "two words", "😀\n"}, []string{"handoff"}, ToolAccessLevel("safe\n\""))
		logger.Instructions("")
		logger.Instructions("instructions\n\a")
		logger.Instructions(strings.Repeat("x", 2000))
		for _, n := range []int{1997, 1998, 1999, 2000, 2001} {
			logger.Instructions(strings.Repeat("x", n) + "😀tail")
		}
		items := []RunItem{
			{Type: RunItemMessage},
			{Type: RunItemMessage, Agent: &Agent{Name: "worker"}, Message: &MessageOutput{Text: "answer\n"}},
			{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "call", Name: "shell"}},
			{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "raw", Name: "tool", Input: json.RawMessage(`{ "x" : 1 }`)}},
			{Type: RunItemToolOutput, ToolOutput: &ToolOutputData{CallID: "call", Content: "result\n", IsError: true}},
			{Type: RunItemCompaction},
			{Type: RunItemCompaction, Compaction: &CompactionData{ID: "compact", EncryptedContent: "secret😀"}},
		}
		items = append(items,
			RunItem{Type: RunItemMessage, Message: &MessageOutput{Text: strings.Repeat("x", 500)}},
			RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{Input: json.RawMessage(`"` + strings.Repeat("x", 298) + `"`)}},
			RunItem{Type: RunItemToolOutput, ToolOutput: &ToolOutputData{Content: strings.Repeat("x", 300)}},
		)
		for _, kind := range []RunItemType{3, 4, 5, 6, -1, 9223372036854775807} {
			items = append(items, RunItem{Type: kind})
		}
		for _, n := range []int{497, 498, 499, 500, 501} {
			items = append(items, RunItem{Type: RunItemMessage, Message: &MessageOutput{Text: strings.Repeat("x", n) + "😀tail"}})
		}
		for _, n := range []int{296, 297, 298, 299, 300, 301} {
			text := strings.Repeat("x", n) + "😀tail"
			items = append(items, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "long", Name: "tool", Input: json.RawMessage(`"` + text + `"`)}}, RunItem{Type: RunItemToolOutput, ToolOutput: &ToolOutputData{CallID: "long", Content: text}})
		}
		logger.InputItems(nil)
		logger.InputItems(items)
		logger.ToolExec("shell\n", "id", -7)
		logger.ToolResult("tool", "id", false, "")
		logger.ToolResult("tool", "id", true, "line\n")
		logger.ToolResult("tool", "id", false, strings.Repeat("x", 1000))
		for _, n := range []int{997, 998, 999, 1000, 1001} {
			logger.ToolResult("tool", "long", false, strings.Repeat("x", n)+"😀tail")
		}
		logger.ModelResponse("model", -1, 2, 3, -9223372036854775808, 9223372036854775807, "stop")
		logger.ParallelToolCalls(nil)
		logger.ParallelToolCalls([]string{"tool", "two words"})
		logger.AutoContinue(-2, "reason\n")
		logger.Debugf("debug %s %d", "value", 7)
		logger.Infof("info %s %d\n", "value", 7)
		logger.Warn("warning\n")
		logger.Warnf("warning %s %d", "value", 7)
		logger.Error("error\n")
		logger.Errorf("error %s %d", "value", 7)
		logger.Event(ContentEvent{Type: "session_end", Status: "done"})
		observations = append(observations, map[string]any{"level": int(level), "debug": logger.IsDebug(), "logHex": hex.EncodeToString(output.Bytes())})
	}
	return observations
}
