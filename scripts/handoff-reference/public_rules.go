package main

import (
	"encoding/json"
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
