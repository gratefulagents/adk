// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

func TestNativeUserInputReference(t *testing.T) {
	inputs := []string{
		``, `{`, `null`, `{}`, `[]`, `42`, `"text"`,
		`{"question":"Continue?","choices":["Yes","No"]}`,
		`{"question":" <&>\u2028 ","choices":["<&>",null,""]}`,
		`{"question":null,"choices":null}`,
		`{"question":false,"choices":[true]}`,
		`{"questions":[{"question":"Pick","header":"H","options":[{"label":"A","description":"first"},{"label":"B"}]}]}`,
		`{"questions":[null,{"question":"Next","options":[null]}]}`,
		`{"questions":[{"question":"bad","header":42}],"question":"simple"}`,
		`{"questions":[],"question":"simple"}`,
		`{"summary":"Plan","actions":[{"id":"approve","label":"Yes","mode":"build","style":"primary"}],"recommended":"approve"}`,
		`{"summary":"Plan","actions":[]}`,
		`{"summary":"Plan","actions":null}`,
		`{"summary":"Plan","actions":[null,{}]}`,
		`{"summary":"Plan","recommended":3}`,
		`{"summary":null,"actions":[{"id":null,"label":null,"style":null}]}`,
		`{"QUESTION":"Upper","CHOICES":["Yes"]}`,
		`{"queſtion":"Folded","choiceſ":["Yes"]}`,
		`{"question":"first","question":null,"choices":["one"],"choices":null}`,
		`{"question":"first","QUESTION":"last","choices":["one"],"choices":["two"]}`,
		`{"summary":"first","SUMMARY":null,"actions":[{"id":"first","ID":null,"label":"first","LABEL":"last"}]}`,
		`{"summary":"ok","summary":false,"summary":"last"}`,
		`{"questions":[{"QUESTION":"Upper","OPTIONS":[{"LABEL":"A","DESCRIPTION":"Desc"}]}]}`,
		`{"actions":[{"id":"a"}],"actions":[{"label":"b"}]}`,
		`{"questions":[{"question":"First","options":[{"label":"A"}]}],"questions":[{"options":[{"description":"B"}]}]}`,
		`{"choices":["old","tail"],"choices":["new"],"choices":[null,null]}`,
		`{"actions":[{"id":"a"},{"id":"tail"}],"actions":[null],"actions":[{"label":"b"},null]}`,
		`{"actions":[{"id":"a"}],"actions":[],"actions":[{"label":"b"}]}`,
		`{"actions":[{"id":"a"}],"actions":null,"actions":[{"label":"b"}]}`,

		`{"question":"\ud800","choices":["\udfff","\ud800\udc00"]}`,
		`{"question":"\ud800\ud800\udc00"}`,
		`{"question":"\\ud800"}`,
		`{"question":"\uDFFF\uD800"}`,
		`{"question":"\ud800\uZZZZ"}`,
		`{"summary":"\ud800","actions":[{"id":"\udfff","label":"\uD800\uDC00"}]}`,

		`{"summary":true}`, `{"actions":false}`, `{"choices":[]}`,
	}
	for _, invalid := range [][]byte{{0xff}, {0xe2, 0x82}, {0xed, 0xa0, 0x80}, {0xf0, 0x80, 0x80, 0x80}} {
		inputs = append(inputs, `{"question":"`+string(invalid)+`","choices":["`+string(invalid)+`"]}`, `{"unknown":"`+string(invalid)+`"}`)
	}
	var cases []map[string]any
	for _, input := range inputs {
		raw := json.RawMessage(input)
		summary, actions := ExtractPresentPlanData(raw)
		record := map[string]any{"input": input, "input_hex": hex.EncodeToString([]byte(input)), "question": ExtractAskUserQuestion(raw), "choices": ExtractAskUserChoices(raw), "summary": summary, "actions": actions, "action_bytes": string(actions), "choice_bytes": string(ExtractAskUserChoices(raw))}
		var pauses []map[string]any
		for _, name := range []string{"AskUserQuestion", "present_plan", "other"} {
			for _, final := range []string{"", " fallback \n"} {
				items := []RunItem{{Type: RunItemToolCall, ToolCall: &ToolCallData{Name: name, Input: raw}}}
				pauses = append(pauses, map[string]any{"name": name, "final": final, "pause": DetectUserInputPause(items, final)})
			}
		}
		record["pauses"] = pauses
		cases = append(cases, record)
	}
	var caps []map[string]any
	for _, value := range []int{-9223372036854775808, -12, -1, 0, 1, 123, 9223372036854775807} {
		caps = append(caps, map[string]any{"value": value, "text": BuildAutoTurnCapPrompt(value)})
	}
	result := map[string]any{"caps": caps, "cases": cases, "marshal_nil": string(MarshalQuickActions()), "marshal_empty": string(MarshalQuickActions([]QuickAction{}...)), "cap": BuildAutoTurnCapPrompt(-12)}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
