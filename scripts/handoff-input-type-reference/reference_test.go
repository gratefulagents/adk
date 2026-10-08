// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"
	"testing"
)

func TestNativeHandoffInputTypeReference(t *testing.T) {
	cases := []map[string]any{}
	original := log.Writer()
	defer log.SetOutput(original)
	for _, streamed := range []bool{false, true} {
		for _, name := range []string{"no_type", "shape_valid", "shape_invalid", "json_null", "parse_accept", "parse_reject", "no_callback", "missing", "invalid_json"} {
			var logs bytes.Buffer
			log.SetOutput(&logs)
			raw := `{"reason":"billing"}`
			if name == "no_type" || name == "shape_invalid" {
				raw = `"billing"`
			}
			if name == "json_null" {
				raw = `null`
			}
			if name == "missing" {
				raw = ""
			}
			if name == "invalid_json" {
				raw = "{"
			}
			target := &Agent{Name: "expert"}
			h := NewHandoff(target)
			events := []string{}
			parserInputs := []string{}
			callbackInputs := []string{}
			if name != "no_type" {
				h.InputType = NewOutputSchema("handoff", json.RawMessage(`{"type":"object","properties":{"reason":{"type":"string"}},"required":["reason"]}`))
			}
			custom := name == "parse_accept" || name == "parse_reject" || name == "no_callback" || name == "missing"
			if custom {
				h.InputType.ParseFn = func(input string) (any, error) {
					events = append(events, "parser")
					parserInputs = append(parserInputs, input)
					if name == "parse_reject" {
						return nil, fmt.Errorf("private parse failure")
					}
					return map[string]any{"transformed": true}, nil
				}
			}
			if name != "no_callback" {
				h.OnHandoff = func(_ *RunContext, input json.RawMessage) {
					events = append(events, "callback")
					callbackInputs = append(callbackInputs, string(input))
				}
			}
			source := &Agent{Name: "router", Handoffs: []*Handoff{h}}
			model := &subagentToolMockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "h1", Name: h.ToolName, Input: json.RawMessage(raw)}}}}, {Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "done"}}}}}}
			runner := NewRunnerWithModel(model)
			var result *RunResult
			var err error
			if streamed {
				s := runner.RunStreamed(context.Background(), source, nil, RunConfig{MaxTurns: 3})
				for range s.Events {
				}
				result, err = s.FinalResult(), s.Err()
			} else {
				result, err = runner.Run(context.Background(), source, nil, RunConfig{MaxTurns: 3})
			}
			if err != nil {
				t.Fatal(err)
			}
			warned := strings.Contains(logs.String(), "input failed schema validation")
			if result.LastAgent != target || result.FinalText() != "done" || warned != (name == "parse_reject" || name == "invalid_json") {
				t.Fatalf("%s: result=%v logs=%s", name, result, logs.String())
			}
			cases = append(cases, map[string]any{"scenario": name, "streamed": streamed, "raw": raw, "schema": json.RawMessage(model.requests[0].Tools[0].InputSchema()), "custom_parser": custom, "has_type": h.InputType != nil, "has_callback": h.OnHandoff != nil, "events": events, "parser_inputs": parserInputs, "callback_inputs": callbackInputs, "warning": warned, "last_agent": result.LastAgent.Name, "final_text": result.FinalText(), "native_value_representable": name != "missing" && name != "invalid_json"})
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
