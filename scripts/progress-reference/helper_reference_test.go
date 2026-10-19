// SPDX-License-Identifier: GPL-3.0-only
package agent

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
)

type helperCase struct {
	Name       string `json:"name"`
	Function   string `json:"function"`
	Input      string `json:"input,omitempty"`
	InputBytes []int  `json:"input_bytes,omitempty"`
	Limit      int    `json:"limit"`
	Expected   any    `json:"expected"`
}

func TestNativeProgressHelperReference(t *testing.T) {
	cases := []helperCase{}
	add := func(name, function, input string, limit int) {
		cases = append(cases, helperCase{Name: name, Function: function, Input: input, Limit: limit})
	}
	for _, fn := range []struct{ name, key string }{
		{"ExtractFilePath", "file_path"}, {"ExtractPath", "path"}, {"ExtractGrepPattern", "pattern"}, {"ExtractGlobPattern", "pattern"}, {"ExtractBashCommand", "command"},
	} {
		for _, test := range []struct{ name, input string }{
			{"simple", `{"` + fn.key + `":"src/世界.rs"}`},
			{"null_object", `null`}, {"empty_object", `{}`}, {"invalid_syntax", `{"` + fn.key + `":"x",}`},
			{"null_scalar", `{"` + fn.key + `":null}`}, {"wrong_type", `{"` + fn.key + `":42}`},
			{"uppercase_alias", `{"` + strings.ToUpper(fn.key) + `":"upper"}`},
			{"duplicate", `{"` + fn.key + `":"first","` + fn.key + `":"last"}`},
			{"duplicate_null", `{"` + fn.key + `":"first","` + fn.key + `":null}`},
			{"duplicate_error_then_valid", `{"` + fn.key + `":3,"` + fn.key + `":"last"}`},
			{"huge_unknown_number", `{"unknown":1e1000,"` + fn.key + `":"kept"}`},
			{"surrogate", `{"` + fn.key + `":"a\ud800b\udc00c"}`},
			{"paired_surrogate", `{"` + fn.key + `":"\ud83d\ude00"}`},
			{"nested_unknown", `{"unknown":` + strings.Repeat("[", 140) + "0" + strings.Repeat("]", 140) + `,"` + fn.key + `":"kept"}`},
		} {
			add(fn.name+"_"+test.name, fn.name, test.input, 0)
		}
	}
	for _, test := range []struct{ name, input string }{
		{"simple", `{"operation":"hover","filePath":"src/main.rs"}`},
		{"partial_bad_operation", `{"operation":3,"filePath":"kept.rs"}`},
		{"partial_bad_file", `{"operation":"hover","filePath":false}`},
		{"duplicate_type_error", `{"operation":3,"operation":"references","filePath":"x.rs"}`},
		{"syntax_error_discards_partial", `{"operation":"hover","filePath":`},
		{"case_alias", `{"OPERATION":"definition","FILEPATH":"x"}`},
		{"null", `null`},
	} {
		add("lsp_"+test.name, "ExtractLSPOperation", test.input, 0)
	}
	for _, test := range []struct{ name, input string }{
		{"all_fields", `{"description":"  explicit\nname  ","prompt":"raw prompt","subagent_type":"worker","model":"m","isolation":"worktree"}`},
		{"fallback", `{"prompt":"  fallback\n` + strings.Repeat("界", 90) + `  "}`},
		{"description_limit", `{"description":"` + strings.Repeat("é", 90) + `"}`},
		{"type_error", `{"description":9,"prompt":"fallback"}`},
		{"null", `null`}, {"empty", `{}`}, {"syntax", `{`},
		{"case_alias", `{"DESCRIPTION":"name","SUBAGENT_TYPE":"worker","PROMPT":"p"}`},
		{"unicode_fold", `{"deſcription":"long-s alias","iſolation":"none"}`},
		{"unknown_huge", `{"unknown":1e1000,"description":"known"}`},
	} {
		add("agent_"+test.name, "ExtractAgentMeta", test.input, 0)
	}
	for _, test := range []struct{ name, input string }{
		{"simple", `{"question":"Continue?"}`},
		{"structured", `{"questions":[{"question":"Choose?","header":"ignored","options":[{"label":"yes","description":"continue"},{"label":"no"}]},{"question":"Next?"}]}`},
		{"empty_questions_fallback", `{"questions":[],"question":"fallback"}`},
		{"wrong_questions_fallback", `{"questions":4,"question":"fallback"}`},
		{"unrecognized", `{"other":"raw"}`}, {"null", `null`}, {"syntax", `{`},
		{"case_alias", `{"QUESTION":"Upper?"}`},
		{"surrogate", `{"question":"\ud800"}`},
	} {
		add("question_"+test.name, "ExtractAskUserQuestion", test.input, 0)
	}
	for _, function := range []string{"Truncate", "TruncateBytes", "TruncateMiddle"} {
		for _, input := range []struct{ name, text string }{
			{"ascii", "  alpha\nbeta gamma  "}, {"unicode", "  α界😀\n終 tail  "}, {"long", strings.Repeat("α界😀a", 30)},
		} {
			for _, limit := range []int{-1, 0, 1, 2, 3, 4, 7, 12, 34, 35, 36, 37, 40, 80, 200} {
				// JSON encoding the pair makes deterministic, unambiguous case names.
				suffix, _ := json.Marshal(limit)
				add(function+"_"+input.name+"_"+string(suffix), function, input.text, limit)
			}
		}
	}
	for _, depth := range []int{9999, 10000} {
		input := `{"unknown":` + strings.Repeat("[", depth) + "0" + strings.Repeat("]", depth) + `,"path":"kept"}`
		suffix, _ := json.Marshal(depth)
		add("path_depth_"+string(suffix), "ExtractPath", input, 0)
	}
	for _, function := range []string{"ExtractFilePath", "ExtractAgentMeta", "ExtractAskUserQuestion"} {
		input := []byte("{\"file_path\":\"a\xff\xfe.rs\",\"description\":\"a\xff\xfe\",\"question\":\"a\xff\xfe\"}")
		bytes := make([]int, len(input))
		for i, b := range input {
			bytes[i] = int(b)
		}
		cases = append(cases, helperCase{Name: function + "_invalid_utf8", Function: function, InputBytes: bytes})
	}
	for i := range cases {
		c := &cases[i]
		t.Run(c.Name, func(t *testing.T) {
			input := []byte(c.Input)
			if c.InputBytes != nil {
				input = make([]byte, len(c.InputBytes))
				for i, b := range c.InputBytes {
					input[i] = byte(b)
				}
			}
			switch c.Function {
			case "ExtractFilePath":
				c.Expected = ExtractFilePath(input)
			case "ExtractPath":
				c.Expected = ExtractPath(input)
			case "ExtractGrepPattern":
				c.Expected = ExtractGrepPattern(input)
			case "ExtractGlobPattern":
				c.Expected = ExtractGlobPattern(input)
			case "ExtractBashCommand":
				c.Expected = ExtractBashCommand(input)
			case "ExtractAskUserQuestion":
				c.Expected = ExtractAskUserQuestion(input)
			case "ExtractAgentMeta":
				c.Expected = ExtractAgentMeta(input)
			case "ExtractLSPOperation":
				op, path := ExtractLSPOperation(input)
				c.Expected = map[string]string{"operation": op, "file_path": path}
			case "Truncate":
				c.Expected = Truncate(string(input), c.Limit)
			case "TruncateBytes":
				c.Expected = TruncateBytes(string(input), c.Limit)
			case "TruncateMiddle":
				c.Expected = TruncateMiddle(string(input), c.Limit)
			default:
				t.Fatalf("unhandled function %s", c.Function)
			}
		})
	}
	output, err := json.Marshal(map[string]any{"cases": cases})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("PROGRESS_HELPERS_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}
