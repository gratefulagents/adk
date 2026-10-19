// SPDX-License-Identifier: GPL-3.0-only
package anthropic

import (
	"encoding/json"
	"os"
	"testing"
)

func TestNativeAssemblerReference(t *testing.T) {
	cases := map[string][]string{
		"empty": {},
		"encrypted-without-message-start": {
			`{"type":"content_block_start","index":0,"content_block":{"type":"compaction","id":"comp_1"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"compaction_encrypted_content","encrypted_content":"opaque"}}`,
			`{"type":"content_block_stop","index":0}`,
		},
		"partial-replacement-and-stop-order": {
			`{"type":"content_block_start","index":9,"content_block":{"type":"text","text":"ignored","phase":"commentary"}}`,
			`{"type":"content_block_delta","index":9,"delta":{"type":"text_delta","text":"first"}}`,
			`{"type":"content_block_start","index":-2,"content_block":{"type":"text"}}`,
			`{"type":"content_block_delta","index":-2,"delta":{"type":"text_delta","text":"second"}}`,
			`{"type":"content_block_stop","index":-2}`,
			`{"type":"content_block_stop","index":-2}`,
			`{"type":"content_block_start","index":9,"content_block":{"type":"text","phase":"final_answer"}}`,
			`{"type":"content_block_delta","index":9,"delta":{"type":"text_delta","text":"replacement <>&"}}`,
			`{"type":"content_block_stop","index":9}`,
			`{"type":"content_block_delta","index":9,"delta":{"type":"text_delta","text":"ignored-after-stop"}}`,
			`{"type":"content_block_stop","index":22}`,
		},
		"usage-and-message-metadata": {
			`{"type":"message_delta","delta":{"stop_reason":"end_turn","end_turn":false},"usage":{"output_tokens":7}}`,
			`{"type":"message_start","message":{"id":"one","type":"not-copied","role":"assistant","model":"model","usage":{"input_tokens":1200,"cache_read_input_tokens":800,"cache_creation_input_tokens":50}}}`,
			`{"type":"message_delta","delta":{"stop_reason":"","end_turn":true},"usage":{"output_tokens":42}}`,
			`{"type":"message_delta","usage":{"input_tokens":-1,"output_tokens":0,"cache_read_input_tokens":-3}}`,
			`{"type":"message_start","message":{"id":"two","content":[{"type":"text","text":"ignored"}]}}`,
			`{"type":"message_start","message":null}`,
			`{"type":"unknown"}`, `{"type":"message_stop"}`, `{"type":"ping"}`,
		},
	}
	for _, kind := range []string{"text", "thinking", "redacted_thinking", "compaction", "tool_use", "unknown"} {
		block, _ := json.Marshal(map[string]any{"type": kind, "text": "initial text", "thinking": "initial thinking", "signature": "sig", "data": "data", "encrypted_content": "initial cipher", "id": "id", "name": "name", "phase": "phase", "content": "summary ", "created_by": "provider", "input": map[string]any{"ignored": true}})
		cases["block-"+kind] = []string{
			`{"type":"content_block_start","index":0,"content_block":` + string(block) + `}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"text"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"thought"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"+signature"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"reasoning_encrypted_content","encrypted_content":"reason cipher"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"part1 ","encrypted_content":"compact cipher"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"compaction_delta","content":"part2"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{ \"z\":1e+02, \"a\":2.00, \"z\":3 }"}}`,
			`{"type":"content_block_delta","index":0,"delta":{"type":"unknown"}}`,
			`{"type":"content_block_stop","index":0}`,
		}
	}
	for _, input := range []string{"", "null", "[1,2]", "false", "{", "1e+02"} {
		delta, _ := json.Marshal(map[string]any{"type": "content_block_delta", "index": 0, "delta": map[string]any{"type": "input_json_delta", "partial_json": input}})
		cases["tool-input-"+input] = []string{
			`{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"tool","input":{"ignored":true}}}`,
			string(delta), `{"type":"content_block_stop","index":0}`,
		}
	}
	output := map[string]any{}
	for name, events := range cases {
		a := NewStreamAssembler()
		states := []map[string]any{}
		capture := func() {
			raw, err := json.Marshal(a.Response())
			states = append(states, map[string]any{"ok": err == nil, "json": string(raw)})
		}
		capture()
		for _, raw := range events {
			var event StreamEvent
			if err := json.Unmarshal([]byte(raw), &event); err != nil {
				t.Fatal(err)
			}
			a.Add(event)
			capture()
		}
		output[name] = map[string]any{"events": events, "states": states}
	}
	raw, err := json.MarshalIndent(output, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("ASSEMBLER_OUTPUT"), raw, 0600); err != nil {
		t.Fatal(err)
	}
}
