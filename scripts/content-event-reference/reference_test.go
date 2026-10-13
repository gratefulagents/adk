// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"math"
	"os"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestNativeContentEventReference(t *testing.T) {
	cases := []ContentEvent{
		{},
		{Type: "unknown"},
		{Type: "tool_end", AgentName: "researcher", Tool: "Bash", Status: "failed", IsError: true, FailureKind: "execution", Reason: " reason\nsecond", Message: " message\nsecond", Output: "exit 2\nbad"},
		{Type: "tool_end", Output: "successful output is omitted"},
		{Type: "tool_end", IsError: true},
		{Type: "event", Reason: " \t ", Message: "\n", IsError: true, Output: " "},
		{Type: "event", Reason: " first \r\nsecond ", Message: " third \nlast "},
		{Type: "event", AgentName: " raw agent ", Tool: "raw\nname", Status: " raw status ", FailureKind: "raw\nfailure"},
		{Type: "event", Reason: "\u0085界😀\u00a0", Message: "\ufeffpreserved"},
		{Type: "event", Reason: strings.Repeat("long", 1000), IsError: true, Output: strings.Repeat("output", 1000)},
	}
	observations := []map[string]any{}
	for _, event := range cases {
		observations = append(observations, map[string]any{"event": event, "compact": CompactContentEvent(event)})
	}
	fields := []map[string]any{}
	nonfinite := []map[string]any{}
	eventType := reflect.TypeOf(ContentEvent{})
	escaped := "<tag>&\"\\界😀\u0085\u00a0\u2028\u2029"
	for control := rune(0); control < 32; control++ {
		escaped += string(control)
	}
	for i := 0; i < eventType.NumField(); i++ {
		field := eventType.Field(i)
		values := []reflect.Value{reflect.Zero(field.Type)}
		switch field.Type.Kind() {
		case reflect.String:
			values = append(values, reflect.ValueOf(escaped))
		case reflect.Bool:
			values = append(values, reflect.ValueOf(true))
		case reflect.Int, reflect.Int32, reflect.Int64:
			maximum := int64(uint64(1)<<(field.Type.Bits()-1) - 1)
			for _, number := range []int64{1, -1, -maximum - 1, maximum} {
				value := reflect.New(field.Type).Elem()
				value.SetInt(number)
				values = append(values, value)
			}
		case reflect.Float64:
			for _, number := range []float64{
				math.Copysign(0, -1), 1, -1, 0.125,
				math.Nextafter(1e-6, 0), 1e-6, math.Nextafter(1e-6, 1),
				math.Nextafter(1e21, 0), 1e21, math.Nextafter(1e21, math.Inf(1)),
				-1e-7, -1e21, 1.2345678901234567e100,
				math.SmallestNonzeroFloat64, -math.SmallestNonzeroFloat64,
				math.MaxFloat64, -math.MaxFloat64,
			} {
				values = append(values, reflect.ValueOf(number))
			}
			for _, number := range []float64{math.NaN(), math.Inf(1), math.Inf(-1)} {
				event := reflect.New(eventType).Elem()
				event.Field(i).SetFloat(number)
				_, err := json.Marshal(event.Interface())
				if err == nil {
					t.Fatalf("%s accepted %v", field.Name, number)
				}
				nonfinite = append(nonfinite, map[string]any{
					"name": field.Name, "json_name": strings.Split(field.Tag.Get("json"), ",")[0],
					"value": strconv.FormatFloat(number, 'g', -1, 64), "error": err.Error(),
				})
			}
		case reflect.Slice:
			if field.Type != reflect.TypeOf([]string{}) {
				t.Fatalf("uncovered slice type: %s", field.Type)
			}
			values = append(values, reflect.ValueOf([]string{}), reflect.ValueOf([]string{"", escaped, "second"}))
		case reflect.Struct:
			if field.Type != reflect.TypeOf(time.Time{}) {
				t.Fatalf("uncovered struct type: %s", field.Type)
			}
			values = append(values,
				reflect.ValueOf(time.Date(2025, 2, 3, 4, 5, 6, 123456789, time.FixedZone("east", 5*3600+30*60))),
				reflect.ValueOf(time.Date(2025, 2, 3, 4, 5, 6, 123400000, time.FixedZone("west", -7*3600))),
			)
		default:
			t.Fatalf("uncovered field type: %s", field.Type)
		}
		serialized := []string{}
		for _, value := range values {
			event := reflect.New(eventType).Elem()
			event.Field(i).Set(value)
			data, err := json.Marshal(event.Interface())
			if err != nil {
				t.Fatalf("%s: %v", field.Name, err)
			}
			serialized = append(serialized, string(data))
		}
		fields = append(fields, map[string]any{
			"name": field.Name, "json_name": strings.Split(field.Tag.Get("json"), ",")[0],
			"go_type": field.Type.String(), "cases": serialized,
		})
	}
	lines := []string{
		"", " \t\n", "not-json\nsecond", "null", "{}", "[]", "true", "1", `"text"`,
		`{"type":"x"}`, `{"type":""}`, `{"type":null}`, `{"TYPE":"event","MeSSage":"text"}`,
		`{"type":"a","type":"b"}`, `{"type":"a","type":null}`, `{"type":"a","type":3,"type":"b"}`,
		`{"type":"event","ſtatuſ":"ready","coſt_known":true}`,
		`{"type":"event","message":"\ud800"}`, `{"type":"event","message":"\udfff"}`,
		`{"type":"event","message":"\ud800\udfff"}`, `{"type":"event","message":"\ud800x\ud800"}`,
		`{"type":"event","tools":["a","b"],"TOOLS":[null]}`,
		`{"type":"event","tools":["a","b"],"tools":["c"],"tools":[null,null]}`,
		`{"type":"event","tools":["a","b"],"tools":[],"tools":[null]}`,
		`{"type":"event","tools":["a","b"],"tools":null,"tools":[null]}`,
		`{"type":"event","ts":"2024-01-01T00:00:00Z","ts":null}`,
		`{"type":"event","ts":"2024-01-01T00:00:00\u005a"}`,
		`{"type":"event","unknown":{"a":[null,true,1e999,"\ud800"]}}`,
		`{"type":"event"} null`, `{"type":"event",}`, `{"type":"event","message":"bad` + "\n" + `string"}`,
	}
	for _, timestamp := range []string{
		"0000-01-01T00:00:00Z", "2024-02-29T1:02:03Z", "2024-01-01T00:00:00,123Z",
		"2024-01-01T00:00:00.123456789123Z", "2024-01-01T00:00:00+24:00",
		"2024-01-01T00:00:00-24:60", "2024-01-01T00:00:00+00:60",
		"2024-01-01T00:00:00+25:00", "2024-01-01T00:00:00+00:61",
		"2023-02-29T00:00:00Z", "2024-01-01T00:00:60Z", "2024-01-01T00:00:00.Z",
	} {
		encoded, _ := json.Marshal(timestamp)
		lines = append(lines, `{"type":"event","ts":`+string(encoded)+`}`)
	}
	for _, field := range fields {
		name := field["json_name"].(string)
		for _, canonical := range field["cases"].([]string) {
			if name == "type" {
				continue
			}
			lines = append(lines, canonical[:len(canonical)-1]+`,"type":"event"}`)
			var decoded map[string]json.RawMessage
			if err := json.Unmarshal([]byte(canonical), &decoded); err != nil {
				t.Fatal(err)
			}
			if value, exists := decoded[name]; exists {
				lines = append(lines, `{"type":"event","`+name+`":`+string(value)+`,"`+strings.ToUpper(name)+`":null}`)
			}
		}
		if name == "type" {
			continue
		}
		for _, value := range []string{`null`, `"string"`, `true`, `1`, `-0`, `1.0`, `1e0`, `1e999`, `[]`, `{}`, `[null,"x"]`} {
			lines = append(lines, `{"type":"event","`+name+`":`+value+`}`)
		}
	}
	for _, depth := range []int{128, 9999, 10000, 10001} {
		lines = append(lines, `{"type":"event","unknown":`+strings.Repeat("[", depth)+`0`+strings.Repeat("]", depth)+`}`)
	}
	parsed := []map[string]any{}
	for _, line := range lines {
		event, ok := ParseContentEventLine(line)
		row := map[string]any{"line": line, "ok": ok, "compact": CompactContentEventLine(line)}
		if ok {
			encoded, err := json.Marshal(event)
			row["marshal_ok"] = err == nil
			row["timestamp"] = event.Timestamp.Format(time.RFC3339Nano)
			if err == nil {
				row["event_json"] = string(encoded)
			}
		}
		parsed = append(parsed, row)
	}
	data, err := json.MarshalIndent(map[string]any{"cases": observations, "fields": fields, "nonfinite_rejections": nonfinite, "lines": parsed}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
