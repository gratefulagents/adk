// SPDX-License-Identifier: GPL-3.0-only
package events

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"strings"
	"testing"
	"unicode/utf8"
)

type nativeChunk struct {
	Text   string `json:"text"`
	Repeat int    `json:"repeat"`
	Tail   string `json:"tail"`
}

func (c nativeChunk) MarshalJSON() ([]byte, error) {
	row := map[string]any{"repeat": c.Repeat, "tail": c.Tail}
	if utf8.ValidString(c.Text) {
		row["text"] = c.Text
	} else {
		bytes := []int{}
		for _, b := range []byte(c.Text) {
			bytes = append(bytes, int(b))
		}
		row["bytes"] = bytes
	}
	return json.Marshal(row)
}

func TestNativeLineWriterReference(t *testing.T) {
	const limit = 16 * 1024 * 1024
	child := `{"type":"tool_end","parent_call_id":"parent","tool_use_id":"child","tool":"Bash","output":"result","tool_duration_ms":42}`
	subagent := `{"type":"subagent_status","task_id":"task","parent_call_id":"parent","subagent_type":"worker","subagent_waiting_on":["dependency"]}`
	cases := []struct {
		name   string
		chunks []nativeChunk
	}{
		{"empty", []nativeChunk{{"", 1, ""}, {"", 1, ""}}},
		{"ordered", []nativeChunk{{"plain\n" + child + "\n" + subagent + "\n", 1, ""}}},
		{"partial", []nativeChunk{{`{"type":"tool_end","is_error":true,"output":"hel`, 1, ""}, {`lo"}`, 1, ""}, {"\n", 1, ""}}},
		{"empty_write_dispatches", []nativeChunk{{child, 1, ""}, {"", 1, ""}}},
		{"drop_retains_partial", []nativeChunk{{subagent, 1, ""}}},
		{"crlf", []nativeChunk{{"first\r\n\r\n", 1, ""}, {"second\r", 1, ""}, {"\n", 1, ""}}},
		{"cr_is_removed_before_continuation", []nativeChunk{{"first\r", 1, ""}, {"second\n", 1, ""}}},
		{"blank", []nativeChunk{{" \t\n\u0085\n\u00a0\n", 1, ""}}},
		{"mixed_partial", []nativeChunk{{child + "\nleft", 1, ""}, {"over\n" + subagent + "\nraw", 1, ""}, {"", 1, ""}}},
		{"malformed_and_nonchild", []nativeChunk{{"null\n{}\n{bad}\n" + `{"type":"tool_end","is_error":true,"output":"bad\nmore"}` + "\n", 1, ""}}},
		{"below_limit_no_lf", []nativeChunk{{"x", limit - 1, ""}, {"", 1, ""}}},
		{"at_limit_no_lf", []nativeChunk{{"x", limit, ""}, {"next\n", 1, ""}}},
		{"at_limit_with_lf", []nativeChunk{{"x", limit - 1, "\n"}}},
		{"over_limit_with_lf", []nativeChunk{{"x", limit, "\n"}}},
		{"prefix_then_oversized", []nativeChunk{{child + "\n", 1, ""}, {"x", limit, ""}}},
		{"oversized_same_write_prefix", []nativeChunk{{"x", limit, "\n"}}},
		{"oversized_partial", []nativeChunk{{"x", limit - 1, ""}, {"x", 1, ""}}},
		{"oversized_same_write_child", []nativeChunk{{"x", limit, "\n"}}},
		{"oversized_valid_child", []nativeChunk{{"x", limit, "\"}\n"}}},
		{"nil_sink", []nativeChunk{{child, 1, ""}, {"\n" + subagent + "\n", 1, ""}}},
	}
	unicode := "界😀"
	cases = append(cases, struct {
		name   string
		chunks []nativeChunk
	}{"split_utf8", []nativeChunk{{unicode[:1], 1, ""}, {unicode[1:4], 1, ""}, {unicode[4:], 1, "\n"}}})
	cases = append(cases, struct {
		name   string
		chunks []nativeChunk
	}{"invalid_utf8", []nativeChunk{{string([]byte{0xff, 0xe2, 0x82}), 1, "\n"}}})
	rows := []map[string]any{}
	for _, tc := range cases {
		emitted := []map[string]any{}
		writer := NewLineWriter(SinkFunc(func(event Event) {
			row := map[string]any{"kind": event.Kind}
			if event.Kind == KindLog {
				row["raw_text_len"] = len(event.Text)
				encoded, _ := json.Marshal(event.Text)
				var rendered string
				if err := json.Unmarshal(encoded, &rendered); err != nil {
					t.Fatal(err)
				}
				row["text_len"] = len(rendered)
				row["text_sha256"] = fmt.Sprintf("%x", sha256.Sum256([]byte(rendered)))
				if len(event.Text) < 1024 {
					row["text"] = event.Text
				}
			}
			if event.Child != nil {
				row["child"] = event.Child
			}
			if event.SubAgent != nil {
				row["subagent"] = event.SubAgent
			}
			emitted = append(emitted, row)
		}))
		if tc.name == "nil_sink" {
			writer = NewLineWriter(nil)
		}
		prefix := ""
		switch tc.name {
		case "oversized_same_write_prefix":
			prefix = "head\n"
		case "oversized_same_write_child":
			prefix = child + "\n"
		case "oversized_valid_child":
			prefix = `{"type":"tool_end","parent_call_id":"parent","is_error":true,"output":"`
		}
		writes := []map[string]any{}
		for _, chunk := range tc.chunks {
			data := prefix + strings.Repeat(chunk.Text, chunk.Repeat) + chunk.Tail
			n, err := writer.Write([]byte(data))
			if err != nil {
				t.Fatal(err)
			}
			writes = append(writes, map[string]any{"accepted": n, "emitted_count": len(emitted)})
		}
		rows = append(rows, map[string]any{"name": tc.name, "prefix": prefix, "nil_sink": tc.name == "nil_sink", "chunks": tc.chunks, "writes": writes, "events": emitted})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": rows}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
