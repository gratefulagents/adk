package main

import (
	"encoding/json"
	"fmt"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/guardrails"
)

type regexClassCase struct {
	Pattern string `json:"pattern"`
	Error   string `json:"error"`
	Matches string `json:"matches"`
}

func regexRepeatScenarios() []regexClassCase {
	patterns := []string{
		`a{999}`, `a{1000}`, `a{1001}`, `a{0,1000}`, `a{0,1001}`,
		`a{1000,}`, `a{1001,}`, `a{1001,1002}`, `a{2,1}`,
		`(a{10}){100}`, `(a{10}){101}`, `(a{100}){10}`, `(a{101}){10}`,
		`(a{10,}){100}`, `(a{10,}){101}`, `(a{0,10}){100}`, `(a{0,10}){101}`,
		`(a{1000})*`, `(a{1000})+`, `(a{1000})?`, `(a{1000}){0}`,
		`((a{1000}){0}){1000}`, `((a{1001}){0}){1000}`,
		`(a{1000}|b){2}`, `(ab{1000}){2}`, `a{1000}b{1000}`,
		`(?:a{10}){100}`, `(?i:a{10}){101}`, `(a{10}){101}?`,
		`\Qa{1001}\E`, `a\{1001\}`, `[{1001}]`,
		`((a{1000}){0,0}){1000}`, `((a{1000}){0,}){1000}`,
		`((a{1000}){1,}){2}`, `((a{1000})?){2}`,
		`((a{10}){10}){10}`, `((a{10}){10}){11}`,
		`(a{1,1000}){2}`, `(a{0,1000}){0}`,
	}
	cases := []regexClassCase{}
	for _, pattern := range patterns {
		observed := regexClassCase{Pattern: pattern}
		_, _, errors := guardrails.ToolGuardrailsFromRules([]sdk.GuardrailRule{{Name: "repeat", Type: "tool-input", Regex: pattern}})
		if len(errors) != 0 {
			observed.Error = errors[0].Error()
		}
		cases = append(cases, observed)
	}
	return cases
}

func regexLiteralScenarios() ([]regexClassCase, []string) {
	inputs := []string{"", "a", "aa", "aaa", "a{", "a{1", "a{01}", "a{1,02}", "a{00,2}", "a{1,}", "a{}", "a{,2}", "a{1, 2}", "a{1,2", "a{١}", "a{-1}", "{01}", "{01}{01}", "a{{", "a{01}}", "a{999999999999x}", "a{1,,2}", "a}b"}
	patterns := []string{`^a{01}$`, `^a{1,02}$`, `^a{00,2}$`, `^a{}$`, `^a{,2}$`, `^a{1, 2}$`, `^a{1,2$`, `^a{1$`, `^a{$`, `^a{١}$`, `^a{-1}$`, `^{01}+$`, `^a{{2}$`, `^a{01}+$`, `^a{999999999999x}$`, `^a{1,,2}$`, `^a}b$`, `^a{0}$`, `^a{1}$`, `^a{1,2}$`, `^a{1,}$`, `^a\{01\}$`, `^\Qa{01}\E$`}
	return regexMembership(patterns, inputs), inputs
}

func regexPerlClassScenarios(inputs []string) []regexClassCase {
	patterns := []string{}
	for _, atom := range []string{`\d`, `\D`, `\s`, `\S`, `\w`, `\W`} {
		for _, negate := range []string{"", "^"} {
			for _, flags := range []string{"", "(?i)"} {
				patterns = append(patterns, fmt.Sprintf("%s^[%s%s]$", flags, negate, atom))
			}
		}
	}
	patterns = append(patterns, `^[\dA]$`, `^[\D-a]$`, `^[]\w]$`, `^[^]\s]$`, `^[[:alpha:]\d]$`, `^[\s\W]$`)
	patterns = append(patterns, `^[\d-\w]$`, `^[\D-]$`, `^[\d[:alpha:]]$`, `^[\w\s]$`, `^[\W\S]$`, `[a-\d]`, `[\b]`, `[\B]`, `[\Q]`, `[\E]`)
	return regexMembership(patterns, inputs)
}

func regexSetLiteralScenarios(inputs []string) []regexClassCase {
	patterns := []string{}
	for _, operator := range []string{"&&", "~~", "||"} {
		for _, format := range []string{"^[a%sb]$", "^[^a%sb]$", "^[]a%s]$", "^[^]a%s]$", "(?i)^[k%ss]$", `^[\d%s\w]$`} {
			patterns = append(patterns, fmt.Sprintf(format, operator))
		}
	}
	patterns = append(patterns, `^[&~|]$`, `^[&~|&~|]$`, `^[[:alpha:]&&[:digit:]]$`)
	return regexMembership(patterns, inputs)
}

func regexClassScenarios() ([]regexClassCase, []string) {
	inputs := []string{}
	for r := rune(0); r < 128; r++ {
		inputs = append(inputs, string(r))
	}
	inputs = append(inputs, "é", "界", "\u0085", "\u00a0", "K", "ſ", "😀", "\n", "a\n", "\r\n", "", "Aa", "12", "_")
	patterns := []string{}
	for _, name := range []string{"alnum", "alpha", "ascii", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space", "upper", "word", "xdigit"} {
		for _, negate := range []string{"", "^"} {
			for _, flags := range []string{"", "(?i)"} {
				patterns = append(patterns, fmt.Sprintf("%s^[[:%s%s:]]$", flags, negate, name))
			}
		}
	}
	patterns = append(patterns, `^[[:alpha:][:digit:]_]+$`, `^[^][:digit:]]$`, `^[][:alpha:]]$`, `^[:alpha:]$`, `^[[:space:]x]$`, `^[[:alpha:]-_]$`)
	return regexMembership(patterns, inputs), inputs
}

func regexMembership(patterns, inputs []string) []regexClassCase {
	cases := []regexClassCase{}
	tool := &sdk.FunctionTool{ToolName: "reader"}
	for _, pattern := range patterns {
		observed := regexClassCase{Pattern: pattern}
		guards, _, errors := guardrails.ToolGuardrailsFromRules([]sdk.GuardrailRule{{Name: "class", Type: "tool-input", Regex: pattern}})
		if len(errors) != 0 {
			observed.Error = errors[0].Error()
		} else {
			for _, input := range inputs {
				result, err := guards[0].Fn(nil, nil, tool, json.RawMessage(input))
				if err != nil {
					panic(err)
				}
				if result.TripwireTriggered {
					observed.Matches += "1"
				} else {
					observed.Matches += "0"
				}
			}
		}
		cases = append(cases, observed)
	}
	return cases
}
