// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strings"
)

func envelopeRaw(raw json.RawMessage) any {
	return map[string]any{"present": raw != nil, "hex": hex.EncodeToString(raw)}
}
func envelopeItemRaw(prefix string, items []LLMRunItemSnapshot, raw map[string]any) {
	for i, item := range items {
		if item.ToolCall != nil {
			raw[fmt.Sprintf("%s/%d/tool_call", prefix, i)] = envelopeRaw(item.ToolCall.Input)
		}
		if item.ToolApproval != nil {
			raw[fmt.Sprintf("%s/%d/tool_approval", prefix, i)] = envelopeRaw(item.ToolApproval.Input)
		}
	}
}
func snapshotEnvelopeCases() []map[string]any {
	inputs := map[string][]string{
		"request": {
			`{}`, `null`, `[]`, `false`,
			`{"agent_name":"agent","model":"model","instructions":"<instruction>","settings":{"temperature":-0.0,"top_p":1e-7,"max_tokens":-3},"input_token_estimate":2,"request_overhead_token_estimate":3,"total_token_estimate":5}`,
			`{"tools":[{"name":"one","input_schema":{"a":1}},{"name":"two","input_schema":[2]}],"TOOLS":[{"name":"new"}],"tools":[null,null]}`,
			`{"tools":[{"input_schema":true}],"tools":[],"tools":[null]}`,
			`{"tools":[{"input_schema":true}],"tools":null,"tools":[null]}`,
			`{"output_schema":{"schema":{"a":1},"strict":true},"OUTPUT_SCHEMA":{"name":"new"}}`,
			`{"output_schema":{"schema":true},"output_schema":null,"output_schema":{}}`,
			`{"input_items":[{"tool_call":{"input":{"a":1}}},{"tool_approval":{"input":[2],"approved":true}}],"INPUT_ITEMS":[{}],"input_items":[null,null]}`,
			`{"input_items":[{"tool_call":{"input":true}}],"input_items":[],"input_items":[null]}`,
			`{"input_items":[{"tool_call":{"input":true}}],"input_items":null,"input_items":[null]}`,
			`{"tools":[{"input_schema":01}],"tools":null}`,
			`{"output_schema":{"schema":[1,]},"output_schema":null}`,
			`{"tools":{},"tools":[]}`,
			`{"unknown":{"input_schema":0},"settings":{"unknown":{"schema":1}}}`,
			"{\"instructions\":\"\xff\",\"tools\":[{\"name\":\"\xff\",\"input_schema\":\"\xff\"}],\"output_schema\":{\"schema\":{\"\xff\":\"\xff\"}}}",
		},
		"response": {
			`{}`, `null`, `[]`, `false`,
			`{"usage":{"input_tokens":2},"USAGE":{"output_tokens":3},"end_turn":false,"texts":["<answer>"],"reasoning":[{"thinking":"thought"}],"reasoning_texts":["reason"],"thinking_texts":["think"],"raw_available":true,"raw_error":"error"}`,
			`{"raw":{"tool_calls":[{"input":0}]},"RAW":[1,2]}`,
			`{"raw":true,"raw":null}`,
			`{"tool_calls":[{"id":"one","input":{"a":1}},{"id":"two","input":[2]}],"TOOL_CALLS":[{}],"tool_calls":[null,null]}`,
			`{"tool_calls":[{"input":true}],"tool_calls":[],"tool_calls":[null]}`,
			`{"tool_calls":[{"input":true}],"tool_calls":null,"tool_calls":[null]}`,
			`{"items":[{"tool_call":{"input":{"a":1}}},{"tool_approval":{"input":[2],"approved":true}}],"ITEMS":[{}],"items":[null,null]}`,
			`{"items":[{"tool_call":{"input":true}}],"items":[],"items":[null]}`,
			`{"raw":01,"raw":null}`,
			`{"tool_calls":[{"input":[1,]}],"tool_calls":null}`,
			`{"tool_calls":{},"tool_calls":[]}`,
			"{\"texts\":[\"\xff\"],\"raw\":\"\xff\",\"tool_calls\":[{\"name\":\"\xff\",\"input\":\"\xff\"}],\"items\":[{\"tool_approval\":{\"input\":{\"\xff\":\"\xff\"}}}]}",
		},
	}
	paths := []struct{ kind, format string }{
		{"request", `{"tools":[{"input_schema":%s}]}`},
		{"request", `{"output_schema":{"schema":%s}}`},
		{"request", `{"input_items":[{"tool_call":{"input":%s}}]}`},
		{"request", `{"input_items":[{"tool_approval":{"input":%s}}]}`},
		{"response", `{"raw":%s}`},
		{"response", `{"tool_calls":[{"input":%s}]}`},
		{"response", `{"items":[{"tool_call":{"input":%s}}]}`},
		{"response", `{"items":[{"tool_approval":{"input":%s}}]}`},
	}
	for _, path := range paths {
		for _, value := range []string{`null`, `false`, `-0`, `1e99999`, `{ "a":1e+02,"a":2,"s":"\ud800" }`, "\"\xff\""} {
			inputs[path.kind] = append(inputs[path.kind], fmt.Sprintf(path.format, value))
		}
		for _, depth := range []int{9995, 9996, 9997, 9998, 9999, 10000} {
			value := strings.Repeat("[", depth) + "0" + strings.Repeat("]", depth)
			inputs[path.kind] = append(inputs[path.kind], fmt.Sprintf(path.format, value))
		}
	}
	cases := []map[string]any{}
	for _, kind := range []string{"request", "response"} {
		for _, input := range inputs[kind] {
			row := map[string]any{"type": kind, "input_hex": hex.EncodeToString([]byte(input))}
			raw := map[string]any{}
			var value any
			var err error
			if kind == "request" {
				var item LLMRequestSnapshot
				err = json.Unmarshal([]byte(input), &item)
				if err == nil {
					envelopeItemRaw("input_items", item.InputItems, raw)
					for i, tool := range item.Tools {
						raw[fmt.Sprintf("tools/%d", i)] = envelopeRaw(tool.InputSchema)
					}
					if item.OutputSchema != nil {
						raw["output_schema"] = envelopeRaw(item.OutputSchema.Schema)
					}
				}
				value = item
			} else {
				var item LLMResponseSnapshot
				err = json.Unmarshal([]byte(input), &item)
				if err == nil {
					envelopeItemRaw("items", item.Items, raw)
					for i, call := range item.ToolCalls {
						raw[fmt.Sprintf("tool_calls/%d", i)] = envelopeRaw(call.Input)
					}
					raw["raw"] = envelopeRaw(item.Raw)
				}
				value = item
			}
			row["decode_ok"] = err == nil
			if err == nil {
				encoded, marshalErr := json.Marshal(value)
				row["marshal_ok"] = marshalErr == nil
				row["marshal_hex"] = hex.EncodeToString(encoded)
				row["raw"] = raw
			}
			cases = append(cases, row)
		}
	}
	return cases
}

func snapshotEnvelopeEncodeCases() []map[string]any {
	values := []json.RawMessage{nil, {}, json.RawMessage(`null`), json.RawMessage(` { "x":1e+02,"x":2 } `), json.RawMessage("\"\xff\""), json.RawMessage(` `), json.RawMessage(`[1,]`)}
	for _, depth := range []int{9999, 10000, 10001} {
		values = append(values, json.RawMessage(strings.Repeat("[", depth)+"0"+strings.Repeat("]", depth)))
	}
	cases := []map[string]any{}
	for _, raw := range values {
		for _, location := range []string{"request_tool", "request_schema", "request_call", "request_approval", "response_raw", "response_call", "response_item_call", "response_approval"} {
			request := LLMRequestSnapshot{}
			response := LLMResponseSnapshot{}
			var value any
			switch location {
			case "request_tool":
				request.Tools = []LLMToolSnapshot{{InputSchema: raw}}
				value = request
			case "request_schema":
				request.OutputSchema = &LLMOutputSchema{Schema: raw}
				value = request
			case "request_call":
				request.InputItems = []LLMRunItemSnapshot{{ToolCall: &LLMToolCall{Input: raw}}}
				value = request
			case "request_approval":
				request.InputItems = []LLMRunItemSnapshot{{ToolApproval: &LLMToolApproval{Input: raw}}}
				value = request
			case "response_raw":
				response.Raw = raw
				value = response
			case "response_call":
				response.ToolCalls = []LLMToolCall{{Input: raw}}
				value = response
			case "response_item_call":
				response.Items = []LLMRunItemSnapshot{{ToolCall: &LLMToolCall{Input: raw}}}
				value = response
			case "response_approval":
				response.Items = []LLMRunItemSnapshot{{ToolApproval: &LLMToolApproval{Input: raw}}}
				value = response
			}
			encoded, err := json.Marshal(value)
			cases = append(cases, map[string]any{"location": location, "raw_nil": raw == nil, "raw_hex": hex.EncodeToString(raw), "marshal_ok": err == nil, "marshal_hex": hex.EncodeToString(encoded)})
		}
	}
	return cases
}
