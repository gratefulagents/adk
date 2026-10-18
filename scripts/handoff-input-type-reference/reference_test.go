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
		for _, name := range []string{"no_type", "shape_valid", "shape_invalid", "json_null", "parse_accept", "parse_reject", "no_callback", "missing", "invalid_json", "unicode", "invalid_schema", "numeric_overflow", "duplicate_overflow"} {
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
			if name == "unicode" {
				raw = `{"reason":"\ud800"}`
			}
			if name == "numeric_overflow" {
				raw = `1e400`
			}
			if name == "duplicate_overflow" {
				raw = `{"n":1e400,"n":1}`
			}
			target := &Agent{Name: "expert"}
			h := NewHandoff(target)
			events := []string{}
			parserInputs := []string{}
			callbackInputs := []string{}
			if name != "no_type" {
				h.InputType = NewOutputSchema("handoff", json.RawMessage(`{"type":"object","properties":{"reason":{"type":"string"}},"required":["reason"]}`))
			}
			if name == "invalid_schema" {
				h.InputType.Schema = json.RawMessage(`{"type":27}`)
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
			if result.LastAgent != target || result.FinalText() != "done" || warned != (name == "parse_reject" || name == "invalid_json" || name == "numeric_overflow" || name == "duplicate_overflow") {
				t.Fatalf("%s: result=%v logs=%s", name, result, logs.String())
			}
			cases = append(cases, map[string]any{"scenario": name, "streamed": streamed, "raw": raw, "schema": json.RawMessage(model.requests[0].Tools[0].InputSchema()), "custom_parser": custom, "has_type": h.InputType != nil, "has_callback": h.OnHandoff != nil, "events": events, "parser_inputs": parserInputs, "callback_inputs": callbackInputs, "warning": warned, "last_agent": result.LastAgent.Name, "final_text": result.FinalText(), "native_value_representable": name != "missing" && name != "invalid_json" && name != "unicode"})
		}
	}
	outputCases := []map[string]any{}
	for _, schema := range []json.RawMessage{json.RawMessage(`{"type":"object","required":["n"],"properties":{"n":{"type":"integer","minimum":0}},"additionalProperties":false}`), json.RawMessage(`{"type":27}`), json.RawMessage(`null`), json.RawMessage(`"schema"`), json.RawMessage(`7`), json.RawMessage(`[]`)} {
		for _, streamed := range []bool{false, true} {
			for _, raw := range []string{`{"n":1}`, `{"n":-1}`, `"text"`, `null`, `[]`, `true`, `not json`, ``, `"\ud800"`, `{"\ud800":"\udc00"}`, `"\ud83d\ude00"`, `"\\ud800"`, `"\ud800\uD800\udc00"`, `9007199254740993`, `1e400`, `{"n":1e400,"n":1}`, `{"$serde_json::private::Number":"1"}`, `{"$serde_json::private::RawValue":"[1]"}`, `-0`, `[0.1,1.0,1e-7,1e21]`} {
				var logs bytes.Buffer
				log.SetOutput(&logs)
				source := &Agent{Name: "output", OutputType: NewOutputSchema("final", schema)}
				model := &subagentToolMockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: raw}}}}}}
				runner := NewRunnerWithModel(model)
				var result *RunResult
				var err error
				if streamed {
					s := runner.RunStreamed(context.Background(), source, nil, RunConfig{MaxTurns: 1})
					for range s.Events {
					}
					result, err = s.FinalResult(), s.Err()
				} else {
					result, err = runner.Run(context.Background(), source, nil, RunConfig{MaxTurns: 1})
				}
				if err != nil {
					t.Fatal(err)
				}
				encoded, err := json.Marshal(result.FinalOutput)
				if err != nil {
					t.Fatal(err)
				}
				outputCases = append(outputCases, map[string]any{"raw": raw, "streamed": streamed, "schema": schema, "schema_json": string(model.requests[0].OutputSchema.Schema), "instructions": model.requests[0].Instructions, "output_json": string(encoded), "warning": strings.Contains(logs.String(), "output schema validation failed")})
			}
		}
	}
	metadata := []map[string]any{}
	for _, name := range []string{"", " \u0085\t ", " final ", "界😀\x00", "x\n\"<&>\\y"} {
		for _, strict := range []bool{false, true} {
			for _, streamed := range []bool{false, true} {
				source := &Agent{Name: "output", OutputType: &OutputSchema{Name: name, Schema: json.RawMessage(`{"type":"object"}`), Strict: strict}}
				model := &subagentToolMockModel{responses: []*ModelResponse{{Items: []RunItem{{Type: RunItemMessage, Message: &MessageOutput{Text: "{}"}}}}}}
				runner := NewRunnerWithModel(model)
				if streamed {
					stream := runner.RunStreamed(context.Background(), source, nil, RunConfig{MaxTurns: 1})
					for range stream.Events {
					}
					if err := stream.Err(); err != nil {
						t.Fatal(err)
					}
				} else if _, err := runner.Run(context.Background(), source, nil, RunConfig{MaxTurns: 1}); err != nil {
					t.Fatal(err)
				}
				if len(model.requests) != 1 {
					t.Fatal("unexpected request count")
				}
				req := model.requests[0]
				metadata = append(metadata, map[string]any{"name": name, "strict": strict, "request_name": req.OutputSchema.Name, "request_strict": req.OutputSchema.Strict, "streamed": streamed, "schema_json": string(req.OutputSchema.Schema), "instructions": req.Instructions})
			}
		}
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases, "output_cases": outputCases, "schema_metadata": metadata}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
