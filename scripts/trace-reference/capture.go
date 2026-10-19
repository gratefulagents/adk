package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	agent "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tracestore"
)

func rawCaptureCases(store *tracestore.FilesystemTraceStore) []map[string]any {
	inputs := []string{
		`null`, `false`, `9007199254740993`, `-0`, `1e+02`, `1e400`,
		`{"n":1e400,"n":1}`, `{"n":9007199254740993,"n":2}`,
		`"\ud800"`, "\"\xff\"", "\"\xe2\x82\"",
		`{"x":"\ud800","n":1e400}`, "{\"x\":\"\xe2\x82\",\"n\":1e400}",
		"{\"x\":\"\xef\xbf\xbd\xe2\x82\",\"n\":1e400}",
		`[1,-0,1e-9,5e-324,-1e-1000,1.7976931348623157e308]`,
		`{"$serde_json::private::Number":"1","$serde_json::private::RawValue":"null"}`,
		`{"PRIVATE_CAPTURE":"PRIVATE_CAPTURE","nested":[{"password":"capture-secret"}]}`,
		`{"password":"capture-secret","n":1e400}`,
		`{"a":"PRIVATE_CAPTURE","a":"retained","z":"PRIVATE_CAPTURE"}`,
		`{`, ``, `[] []`, `{"key":"unterminated}`, `"\uXYZW"`,
	}
	for _, depth := range []int{127, 128, 256, 9998, 9999, 10000, 10001} {
		inputs = append(inputs, strings.Repeat("[", depth)+`"PRIVATE_CAPTURE"`+strings.Repeat("]", depth))
		inputs = append(inputs, strings.Repeat(`{"k":`, depth)+`"PRIVATE_CAPTURE"`+strings.Repeat("}", depth))
	}
	var cases []map[string]any
	for index, input := range inputs {
		runID := fmt.Sprintf("raw-capture-%d", index)
		writer := tracestore.NewTraceWriterWithOptions(store, tracestore.TraceWriterOptions{
			Capture:   tracestore.CaptureFull,
			Redactors: []func(string) string{func(s string) string { return strings.ReplaceAll(s, "PRIVATE_CAPTURE", "[operator]") }},
		})
		must(writer.InitRun(tracestore.RunMetadata{RunID: runID, StartedAt: time.Date(2025, 1, 2, 3, 4, 5, 0, time.UTC)}))
		raw := json.RawMessage(input)
		writer.OnToolStart(&agent.RunContext{}, &agent.Agent{Name: "agent"}, &agent.FunctionTool{ToolName: "Opaque"}, agent.ToolCallData{ID: "call", Name: "Opaque", Input: raw})
		response := &agent.LLMResponseSnapshot{Raw: raw, RawAvailable: true}
		_, marshalErr := json.Marshal(response)
		writer.OnSpanStart(&agent.Span{ID: "span", Data: agent.GenerationSpanData{Response: response}})
		writer.FinalizeRun("completed")
		path, err := store.RunDir(runID)
		must(err)
		field := func(category, key string) string {
			data, err := os.ReadFile(filepath.Join(path, category+".jsonl"))
			must(err)
			// A raw field may itself reach depth 10,000, making the containing
			// encoded record deeper than Go can unmarshal. All preceding fixture
			// fields are fixed; decode only the selected field value.
			prefix := fmt.Sprintf("%q:", key)
			start := strings.Index(string(data), prefix)
			if start < 0 {
				panic("missing capture field")
			}
			var captured json.RawMessage
			must(json.NewDecoder(strings.NewReader(string(data[start+len(prefix):]))).Decode(&captured))
			return hex.EncodeToString(captured)
		}
		cases = append(cases, map[string]any{
			"input_hex":            hex.EncodeToString([]byte(input)),
			"tool_capture_hex":     field("tool_calls", "input"),
			"response_marshal_ok":  marshalErr == nil,
			"response_capture_hex": field("llm_calls", "response"),
		})
	}
	return cases
}
