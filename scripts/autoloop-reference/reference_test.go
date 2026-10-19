// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
)

type autoEvent struct {
	Kind  string `json:"kind"`
	Name  string `json:"name"`
	Input string `json:"input"`
	Text  string `json:"text"`
}
type autoCase struct {
	Name  string        `json:"name"`
	Turns [][]autoEvent `json:"turns"`
	Phase string        `json:"phase"`
}

func autoCall(name, input string) autoEvent { return autoEvent{Kind: "call", Name: name, Input: input} }
func TestNativeAutoloopReference(t *testing.T) {
	cases := []autoCase{}
	noTools := autoCase{Name: "no_tools", Phase: "shipping"}
	for i := 0; i < 6; i++ {
		noTools.Turns = append(noTools.Turns, []autoEvent{})
	}
	cases = append(cases, noTools)
	for _, n := range []int{1, 2, 3} {
		c := autoCase{Name: fmt.Sprintf("cycle_%d", n), Phase: "plan"}
		for i := 0; i < 13; i++ {
			c.Turns = append(c.Turns, []autoEvent{autoCall(fmt.Sprintf("tool%d", i%n), `{"path":"same"}`)})
		}
		cases = append(cases, c)
	}
	for _, mode := range []string{"distinct", "whitespace", "huge_number", "numbers", "malformed", "same_malformed", "missing", "null", "strings"} {
		c := autoCase{Name: mode}
		spellings := []string{"1", "1.0", "1e0", "1E+00", "10e-1", "0.1e1"}
		for i := 0; i < 6; i++ {
			input := fmt.Sprintf(`{"i":%d}`, i)
			switch mode {
			case "whitespace":
				input = strings.Repeat(" ", i) + "{ \"path\" : \"same\" }" + strings.Repeat("\t", 5-i)
			case "huge_number":
				input = strings.Repeat(" ", i) + "1e999" + strings.Repeat("\t", 5-i)
			case "numbers":
				input = spellings[i]
			case "malformed":
				input = fmt.Sprintf("{broken%d", i)
			case "same_malformed":
				input = "{broken"
			case "missing":
				input = ""
			case "null":
				input = "null"
			case "strings":
				input = fmt.Sprintf(`{"s":"%s"}`, strings.Repeat(" ", i))
			}
			c.Turns = append(c.Turns, []autoEvent{autoCall("read_file", input)})
		}
		cases = append(cases, c)
	}
	for _, mode := range []string{"errors", "unicode_errors", "empty_errors", "different_errors", "error_history_retained", "error_precedes_cycle"} {
		c := autoCase{Name: mode, Phase: "review"}
		for i := 0; i < 7; i++ {
			text := "failed"
			switch mode {
			case "unicode_errors":
				text = "  " + strings.Repeat("界", 220) + "  "
			case "empty_errors":
				text = ""
			case "different_errors":
				text = fmt.Sprintf("error%d", i)
			}
			input := fmt.Sprintf(`{"i":%d}`, i)
			if mode == "error_precedes_cycle" {
				input = "{}"
			}
			turn := []autoEvent{autoCall("work", input), {Kind: "error", Text: text}}
			if mode == "error_history_retained" && i == 3 {
				turn = turn[:1]
			}
			c.Turns = append(c.Turns, turn)
		}
		cases = append(cases, c)
	}
	reset := autoCase{Name: "no_tool_reset", Turns: [][]autoEvent{{}, {}, {}, {autoCall("work", "")}, {}, {}}}
	cases = append(cases, reset)
	observations := []map[string]any{}
	for _, c := range cases {
		var tracker AutoTracker
		states := []map[string]any{}
		observe := func() {
			cb := tracker.CheckCircuitBreakers()
			states = append(states, map[string]any{"no_tool_turns": tracker.ConsecutiveNoToolTurns(), "tool_count": tracker.ToolCallCount(), "tripped": cb.Tripped, "reason": cb.Reason, "nudge": BuildSmartNudge(&tracker, c.Phase)})
		}
		observe()
		for _, turn := range c.Turns {
			items := []RunItem{}
			for _, e := range turn {
				switch e.Kind {
				case "call":
					items = append(items, RunItem{Type: RunItemToolCall, ToolCall: &ToolCallData{Name: e.Name, Input: json.RawMessage(e.Input)}})
				case "error":
					items = append(items, RunItem{Type: RunItemToolOutput, ToolOutput: &ToolOutputData{Content: e.Text, IsError: true}})
				}
			}
			tracker.Update(items)
			observe()
		}
		observations = append(observations, map[string]any{"input": c, "states": states})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": observations, "nil_nudge": BuildSmartNudge(nil, "plan"), "default_max_auto_loops": DefaultMaxAutoLoops}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
