// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

func TestNativeItemHelpers(t *testing.T) {
	bodies := []string{
		`null`, `[]`, `[{}]`,
		`[{"Message":{"text":"kept"}},{},{"Message":null}]`,
		`[{"Message":{"text":"hello"}},{"Type":1,"ToolCall":{"Name":"bash"}},{"Message":{"text":"world"}}]`,
		`[{"Message":{"text":"first"}},{"Type":1,"ToolCall":{"Name":"bash"}},{"Message":{"text":"last"}}]`,
		`[{"Type":1,"ToolCall":{"Name":"bash","ID":"1"}},{"Message":{"text":"text"}},{"Type":1,"ToolCall":{"Name":"read","ID":"2"}}]`,
		`[{"Type":1},{"Type":0,"ToolCall":{"name":"ignored"}},{"Type":2,"Message":{"text":"ignored"}},{"Type":99,"Message":{"text":"ignored"}},{"Message":null}]`,
		`[{"Message":{"text":"visible"}},{"Message":{"images":[{"media_type":"image/png","data":"aGk="}]}}]`,
		`[{"Message":{"text":"kept"},"ToolCall":{"name":"ignored"}},{"Type":1,"Message":{"text":"ignored"},"ToolCall":{"name":"kept"}}]`,
		`[{"Type":1,"ToolCall":{"id":"raw","name":"echo","input": { "z": 1e0, "a":2,"a":3 } }},{"Type":1,"ToolCall":{"id":"null","input":null}}]`,
	}
	for _, first := range []string{"", "a", " ", "\n", "界"} {
		for _, middle := range []string{"", "a", " ", "\n", "界"} {
			for _, last := range []string{"", "a", " ", "\n", "界"} {
				body, err := json.Marshal([]RunItem{{Message: &MessageOutput{Text: first}}, {Message: &MessageOutput{Text: middle}}, {Message: &MessageOutput{Text: last}}})
				if err != nil {
					t.Fatal(err)
				}
				bodies = append(bodies, string(body))
			}
		}
	}
	var cases []map[string]any
	for _, body := range bodies {
		var items []RunItem
		if err := json.Unmarshal([]byte(body), &items); err != nil {
			t.Fatal(err)
		}
		calls := Items.ExtractToolCalls(items)
		inputs := []string{}
		for _, call := range calls {
			inputs = append(inputs, hex.EncodeToString(call.Input))
		}
		cases = append(cases, map[string]any{"body": body, "text": Items.ExtractText(items), "last": Items.ExtractLastText(items), "calls": calls, "call_inputs": inputs})
	}
	var rawCases []map[string]any
	for _, input := range []json.RawMessage{nil, {}, []byte{0xff, 0x00}, []byte(`{"incomplete":`), []byte(` {"z":1e0,"a":2,"a":3} `)} {
		calls := Items.ExtractToolCalls([]RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{ID: "raw", Name: "echo", Input: input}}})
		rawCases = append(rawCases, map[string]any{"input": hex.EncodeToString(input), "nil": input == nil, "output": hex.EncodeToString(calls[0].Input), "output_nil": calls[0].Input == nil})
	}
	data, err := json.Marshal(map[string]any{"cases": cases, "raw_cases": rawCases})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("ITEM_HELPERS_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
