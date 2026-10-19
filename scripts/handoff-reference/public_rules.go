package main

import (
	"encoding/json"
	"fmt"
	"regexp"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/guardrails"
)

type publicRuleCase struct {
	Phase  string               `json:"phase"`
	Rule   sdk.GuardrailRule    `json:"rule"`
	Regex  string               `json:"regex"`
	Tool   string               `json:"tool"`
	Text   string               `json:"text"`
	Result *sdk.GuardrailResult `json:"result"`
	Error  string               `json:"error"`
}

func publicRuleScenarios() []publicRuleCase {
	var cases []publicRuleCase
	for _, phase := range []string{"input", "output"} {
		for _, name := range []string{"r1", "line\n\t\"\\\x00", "猫\u200b"} {
			for _, action := range []string{"", "block", "warn", "log", "deny", " WARN "} {
				for _, message := range []string{"", "  ", " custom "} {
					cases = append(cases, publicRuleCase{
						Phase: phase,
						Rule:  sdk.GuardrailRule{Name: name, Type: "ignored", Regex: "[invalid", Action: action, Message: message},
						Regex: "danger", Tool: name + "tool", Text: `{"x":"danger"`,
					})
				}
			}
		}
		for _, pattern := range []string{"*read*", "*reader", "reader", "read*", "*", "**"} {
			cases = append(cases, publicRuleCase{Phase: phase, Rule: sdk.GuardrailRule{Name: "pattern", ToolPattern: pattern}, Regex: "danger", Tool: "reader", Text: "danger"})
		}
		cases = append(cases,
			publicRuleCase{Phase: phase, Rule: sdk.GuardrailRule{Name: "no-match"}, Regex: "danger", Tool: "reader", Text: "safe"},
			publicRuleCase{Phase: phase, Rule: sdk.GuardrailRule{Name: "empty"}, Regex: "^$", Tool: "", Text: ""},
		)
	}
	for i := range cases {
		c := &cases[i]
		tool := &sdk.FunctionTool{ToolName: c.Tool}
		re := regexp.MustCompile(c.Regex)
		var err error
		if c.Phase == "input" {
			c.Result, err = guardrails.MakeToolInputGuardrailFn(c.Rule, re)(nil, nil, tool, json.RawMessage(c.Text))
		} else {
			c.Result, err = guardrails.MakeToolOutputGuardrailFn(c.Rule, re)(nil, nil, tool, sdk.ToolResult{Content: c.Text})
		}
		if err != nil {
			c.Error = err.Error()
		}
	}
	return cases
}

type compiledRuleCase struct {
	Name   string               `json:"name"`
	Rules  []sdk.GuardrailRule  `json:"rules"`
	Input  []builtinGuardResult `json:"input"`
	Output []builtinGuardResult `json:"output"`
	Errors []string             `json:"errors"`
}

func compiledRuleScenarios() []compiledRuleCase {
	invalidRegex := sdk.GuardrailRule{Name: "invalid-regex", Regex: "[", Action: "deny", Type: "agent-input"}
	invalidAction := sdk.GuardrailRule{Name: "invalid-action", Regex: "danger", Action: "deny", Type: "agent-input"}
	invalidType := sdk.GuardrailRule{Name: "invalid-type", Regex: "danger", Action: "block", Type: "agent-input"}
	cases := []compiledRuleCase{
		{Name: "empty", Rules: []sdk.GuardrailRule{}},
		{Name: "all-invalid", Rules: []sdk.GuardrailRule{invalidRegex, invalidAction, invalidType}},
		{Name: "mixed", Rules: []sdk.GuardrailRule{
			{Name: "first", Regex: "danger", Type: "tool-input"},
			invalidRegex,
			{Name: "output-warn", Regex: "danger", Type: "tool-output", Action: "warn", Message: "  "},
			invalidAction,
			{Name: "input-log", Regex: "danger", Type: "tool-input", Action: "log"},
			invalidType,
			{Name: "last", Regex: "danger", Type: "tool-output", Action: "block"},
		}},
		{Name: "normalized"},
		{Name: "patterns", Rules: []sdk.GuardrailRule{
			{Name: "quoted", Regex: `\Q[a&&b]\E`, Type: "tool-input"},
			{Name: "ascii-word", Regex: `\w+`, Type: "tool-output"},
			{Name: "literal-star", Regex: "danger", Type: "tool-input", ToolPattern: "*read*"},
			{Name: "suffix", Regex: "danger", Type: "tool-output", ToolPattern: "*reader"},
		}},
	}
	for i, action := range []string{" \tWaRN\u0085", "blocK", " \n ", " LOG ", " BLOCK "} {
		phase := "tool-input"
		if i%2 != 0 {
			phase = "tool-output"
		}
		cases[3].Rules = append(cases[3].Rules, sdk.GuardrailRule{Name: fmt.Sprintf("normalize-%d", i), Regex: "danger", Action: action, Type: phase})
	}
	tool := &sdk.FunctionTool{ToolName: "reader"}
	for i := range cases {
		c := &cases[i]
		inputs, outputs, errors := guardrails.ToolGuardrailsFromRules(c.Rules)
		for _, err := range errors {
			c.Errors = append(c.Errors, err.Error())
		}
		for _, g := range inputs {
			result, err := g.Fn(nil, nil, tool, json.RawMessage("danger [a&&b]"))
			observed := builtinGuardResult{Name: g.Name, Result: result}
			if err != nil {
				observed.Error = err.Error()
			}
			c.Input = append(c.Input, observed)
		}
		for _, g := range outputs {
			result, err := g.Fn(nil, nil, tool, sdk.ToolResult{Content: "danger [a&&b]"})
			observed := builtinGuardResult{Name: g.Name, Result: result}
			if err != nil {
				observed.Error = err.Error()
			}
			c.Output = append(c.Output, observed)
		}
	}
	return cases
}
