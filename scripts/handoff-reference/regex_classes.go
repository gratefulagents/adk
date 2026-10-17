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

func regexOctalScenarios() ([]regexClassCase, []string) {
	inputs := []string{}
	for r := rune(0); r < 128; r++ {
		inputs = append(inputs, string(r))
	}
	inputs = append(inputs, "", "\u00ff", "\u01ff", "S4", "\x008", "\x009", "\x000", `\123`, `\0`, "83", "777", "界", `\x{ff}`)
	patterns := []string{}
	for _, escape := range []string{`\0`, `\00`, `\000`, `\01`, `\07`, `\10`, `\12`, `\40`, `\77`, `\123`, `\177`, `\377`, `\777`} {
		patterns = append(patterns, "^"+escape+"$", "^["+escape+"]$")
	}
	patterns = append(patterns, `^\1234$`, `^\08$`, `^\09$`, `^\0000$`, `^[\10-\15]$`, `^\\123$`, `^\Q\123\E$`, `\1`, `\7`, `\8`, `[\1]`)
	patterns = append(patterns, `^\x{61}$`, `^\x{ff}$`, `^\p{Han}$`, `^\P{Han}$`, `^[\x{ff}]$`, `^[\p{Han}]$`, `^\x61$`, `^\Q\x{ff}\E$`)
	return regexMembership(patterns, inputs), inputs
}

func regexCaptureScenarios() ([]regexClassCase, []string) {
	inputs := []string{"", "a", "b", "ab", "aa", "aaa", "A", "k", "K", "K", "猫", "(?P<x>a)"}
	patterns := []string{`^(?P<name>a)$`, `^(?<name>a)$`, `^(?P<0>a)$`, `^(?<1a>a)$`, `^(?P<_>a)$`, `^(?P<abc_123>a)$`, `^(?P<x>a)(?P<x>b)$`, `^(?<x>a)|(?<x>b)$`, `^(?P<x>(?P<x>a))$`, `^(?P<x>a){2}$`, `(?i)^(?P<x>k)$`, `^\Q(?P<x>a)\E$`, `(?P<>a)`, `(?<>a)`, `(?P<a-b>a)`, `(?P<猫>a)`, `(?<a.b>a)`, `(?P<name`, `(?'name'a)`, `(?=a)`, `(?P<x>a)\k<x>`, `(?P<x>a)\1`}
	return regexMembership(patterns, inputs), inputs
}

func regexBracketScenarios() ([]regexClassCase, []string) {
	inputs := []string{}
	for r := rune(0); r < 128; r++ {
		inputs = append(inputs, string(r))
	}
	inputs = append(inputs, "", "a]", "[]", "b]", "x]", "&]", "ab]", "[b]", "a]]", "[]]", "猫", "猫]")
	patterns := []string{`^[a[b]]$`, `^[[a-z]$`, `^[[a]b]$`, `^[a-z&&[b]]$`, `^[[:alpha:][a-z]]$`, `^[[:alpha]]$`, `^[[:alpha:xyz]]$`, `^[[.a.]]$`, `^[[]$`, `^[[[]$`, `^[[^]$`, `^[^[a]]$`, `^[[]]$`, `(?i)^[k[s]]$`, `^[\d[a]]$`, `^[[:alpha:][[]]$`, `[[:bogus:]]`, `[[:Alpha:]]`, `[[:alpha:]`, `[[`, `[[:]]`}
	return regexMembership(patterns, inputs), inputs
}

func regexHyphenScenarios() ([]regexClassCase, []string) {
	inputs := []string{"", "猫", "K", "ſ", "猫]"}
	for r := rune(0); r < 128; r++ {
		inputs = append(inputs, string(r), string(r)+"]")
	}
	patterns := []string{}
	for _, prefix := range []string{"", "a", "a-b", `\x00`, `\x00-\x2d`, `\055`, `\d`, `\pL`, "[:alpha:]", "]"} {
		for _, suffix := range []string{"", "a", `\x7f`, `\d`, "[:digit:]", "["} {
			for _, hyphens := range []string{"-", "--", "---", "----"} {
				for _, flags := range []string{"", "(?i)"} {
					patterns = append(patterns, flags+"^["+prefix+hyphens+suffix+"]$")
				}
			}
		}
	}
	patterns = append(patterns, `^[^--a]$`, `^[^a-b--c]$`, `^[a-\pL]$`, `^[\x00-[:digit:]]$`, `^[\x00-[]$`, `^[\x00-\x2d--a]$`, `^[a-z]$`, `^[\-]$`, `^[a-\-]$`, `^[\p{Han}--a]$`)
	return regexMembership(patterns, inputs), inputs
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
