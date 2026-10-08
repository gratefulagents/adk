package main

import (
	"encoding/json"
	"fmt"
	"strings"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/guardrails"
)

type builtinRecipe struct {
	Version  int    `json:"version"`
	Kind     string `json:"kind"`
	Length   int    `json:"length,omitempty"`
	Encoding string `json:"encoding,omitempty"`
}

type builtinGuardResult struct {
	Name   string               `json:"name"`
	Error  string               `json:"error"`
	Result *sdk.GuardrailResult `json:"result"`
}

type builtinCase struct {
	Name         string               `json:"name"`
	Phase        string               `json:"phase"`
	ToolName     string               `json:"tool_name"`
	Recipe       builtinRecipe        `json:"recipe"`
	Params       *string              `json:"params,omitempty"`
	Content      *string              `json:"content,omitempty"`
	Guards       []builtinGuardResult `json:"guards"`
	FinalContent *string              `json:"final_content"`
}

type builtinFixture struct {
	SchemaVersion    int           `json:"schema_version"`
	SDKRevision      string        `json:"sdk_revision"`
	InputGuardNames  []string      `json:"input_guard_names"`
	OutputGuardNames []string      `json:"output_guard_names"`
	Cases            []builtinCase `json:"cases"`
}

// Version 1 uses ASCII-only recipes so native implementations can reproduce
// the bytes without persisting synthetic credential-shaped values.
func builtinRecipeContent(r builtinRecipe) (string, []string, error) {
	if r.Version != 1 || r.Length != 36 {
		return "", nil, fmt.Errorf("unsupported builtin recipe version or length")
	}
	github := "ghp" + "_" + strings.Repeat("a", r.Length)
	aws := "AK" + "IA" + strings.Repeat("A", 16)
	companion := strings.Repeat("z", r.Length)
	pemBegin := "-----BEGIN RSA PRIVATE " + "KEY-----"
	pemEnd := "-----END RSA PRIVATE " + "KEY-----"
	body := strings.Repeat("Q", r.Length)
	gcp := `{"type": "` + "service" + `_account", "private_key_id": "` + companion + `"}`
	var payload string
	var sensitive []string
	switch r.Kind {
	case "github_token":
		payload, sensitive = "token="+github, []string{github}
	case "github_repeated":
		payload, sensitive = "token="+github+"\nagain="+github, []string{github}
	case "pem_closed", "pem_unterminated":
		payload, sensitive = pemBegin+"\n"+body, []string{pemBegin, body}
		if r.Kind == "pem_closed" {
			payload += "\n" + pemEnd
		}
	case "github_and_pem":
		payload, sensitive = "token="+github+"\n"+pemBegin+"\n"+body+"\n"+pemEnd, []string{github, pemBegin, body}
	case "aws_pair", "aws_temporary_pair", "github_and_aws":
		if r.Kind == "aws_temporary_pair" {
			aws = "AS" + "IA" + strings.Repeat("A", 16)
		}
		payload = "AWS_ACCESS_KEY_ID=" + aws + "\nAWS_SECRET_ACCESS_KEY=" + companion
		sensitive = []string{aws, companion}
		if r.Kind == "github_and_aws" {
			payload = "token=" + github + "\n" + payload
			sensitive = append(sensitive, github)
		}
	case "gcp_service_account":
		return gcp, []string{gcp, companion}, nil
	default:
		return "", nil, fmt.Errorf("unknown builtin recipe kind %q", r.Kind)
	}
	return "before\n" + payload + "\nafter", sensitive, nil
}

func builtinScenarios() []builtinCase {
	var cases []builtinCase
	input := func(name, tool, params string) {
		cases = append(cases, builtinCase{Name: name, Phase: "input", ToolName: tool, Recipe: builtinRecipe{Version: 1, Kind: "literal"}, Params: &params})
	}
	for _, tool := range []string{"bash", "BASH", "custom_BaSh_tool", "shell", "ShElL", "exec", "ExEcUtE", "read_file", "sh"} {
		input("name_"+tool+"_safe", tool, `{"command":"printf hello"}`)
		input("name_"+tool+"_destructive", tool, `{"command":"rm -rf /"}`)
		input("name_"+tool+"_malformed", tool, `{"command":`)
	}
	for i, command := range []string{
		"ls -la", "git status", `grep -r "rm -rf /" docs/`, "rm -rf ./build",
		"rm -fr /", "rm -r -f /", "sudo rm -rf /", "env rm -rf /etc", `\rm -rf /`, `"rm" -rf /`,
		`bash -c "rm -rf /"`, "echo hi && rm -rf /", "true; rm -rf /usr", "mkfs.ext4 /dev/sda1", "dd if=/dev/zero of=/dev/sda",
		"rm${IFS}-rf${IFS}/", `r$'m' -rf /`, "echo $(rm -rf /)",
	} {
		params, _ := json.Marshal(struct {
			Command string `json:"command"`
		}{command})
		input(fmt.Sprintf("command_%02d", i), "bash", string(params))
	}
	for _, item := range []struct{ name, params string }{
		{"cmd_only", `{"cmd":"rm -rf /"}`},
		{"command_wins_safe", `{"command":"printf hello","cmd":"rm -rf /"}`},
		{"command_wins_blocked", `{"command":"rm -rf /","cmd":"printf hello"}`},
		{"empty_command_fallback", `{"command":"","cmd":"rm -rf /"}`},
		{"null_command_fallback", `{"command":null,"cmd":"rm -rf /"}`},
		{"empty_object", `{}`}, {"empty_fields", `{"command":"","cmd":""}`},
		{"unknown_field", `{"text":"rm -rf /"}`}, {"null", `null`}, {"array", `[]`},
		{"string", `"hello"`}, {"number", `42`}, {"empty", ``}, {"trailing", `{} trailing`},
		{"command_number", `{"command":42}`}, {"command_bool", `{"command":true}`},
		{"command_array", `{"command":[]}`}, {"command_object", `{"command":{}}`},
		{"cmd_number", `{"cmd":42}`}, {"valid_command_wrong_cmd", `{"command":"printf hello","cmd":42}`},
	} {
		input("params_"+item.name, "bash", item.params)
	}
	for _, content := range []string{"", "before\nordinary output\nafter", "public identifier; no credentials"} {
		cases = append(cases, builtinCase{Name: fmt.Sprintf("output_literal_%d", len(cases)), Phase: "output", ToolName: "read_file", Recipe: builtinRecipe{Version: 1, Kind: "literal"}, Content: &content})
	}
	for _, kind := range []string{"github_token", "github_repeated", "pem_closed", "pem_unterminated", "github_and_pem", "aws_pair", "aws_temporary_pair", "github_and_aws", "gcp_service_account"} {
		for _, phase := range []string{"input", "output"} {
			encoding := "raw"
			if phase == "input" && kind != "gcp_service_account" {
				encoding = "json_text"
			}
			cases = append(cases, builtinCase{Name: phase + "_" + kind, Phase: phase, ToolName: "read_file", Recipe: builtinRecipe{Version: 1, Kind: kind, Length: 36, Encoding: encoding}})
		}
	}
	cases = append(cases, builtinCase{Name: "input_gcp_escaped_json_text", Phase: "input", ToolName: "read_file", Recipe: builtinRecipe{Version: 1, Kind: "gcp_service_account", Length: 36, Encoding: "json_text"}})
	return cases
}

func generateBuiltinGuardrails() ([]byte, error) {
	inputs, outputs := guardrails.BuiltinToolInputGuardrails(), guardrails.BuiltinToolOutputGuardrails()
	fixture := builtinFixture{SchemaVersion: 1, SDKRevision: sdkRevision, Cases: builtinScenarios()}
	for _, g := range inputs {
		fixture.InputGuardNames = append(fixture.InputGuardNames, g.Name)
	}
	for _, g := range outputs {
		fixture.OutputGuardNames = append(fixture.OutputGuardNames, g.Name)
	}
	for i := range fixture.Cases {
		c := &fixture.Cases[i]
		tool := &sdk.FunctionTool{ToolName: c.ToolName}
		var params json.RawMessage
		var result sdk.ToolResult
		var sensitive []string
		if c.Recipe.Kind != "literal" {
			content, fragments, err := builtinRecipeContent(c.Recipe)
			if err != nil {
				return nil, err
			}
			sensitive = fragments
			if c.Phase == "input" {
				if c.Recipe.Encoding == "json_text" {
					params, err = json.Marshal(struct {
						Text string `json:"text"`
					}{content})
					if err != nil {
						return nil, err
					}
				} else {
					params = json.RawMessage(content)
				}
			} else {
				result.Content = content
			}
		} else if c.Phase == "input" {
			params = json.RawMessage(*c.Params)
		} else {
			result.Content = *c.Content
		}
		blocked := false
		if c.Phase == "input" {
			for _, g := range inputs {
				gr, err := g.Fn(nil, nil, tool, params)
				observation := builtinGuardResult{Name: g.Name, Result: gr}
				if err != nil {
					observation.Error = err.Error()
				}
				c.Guards = append(c.Guards, observation)
				if err != nil || (gr != nil && gr.TripwireTriggered) {
					break
				}
			}
		} else {
			for _, g := range outputs {
				gr, err := g.Fn(nil, nil, tool, result)
				observation := builtinGuardResult{Name: g.Name, Result: gr}
				if err != nil {
					observation.Error = err.Error()
				}
				c.Guards = append(c.Guards, observation)
				if err != nil || (gr != nil && gr.TripwireTriggered) {
					blocked = true
					break
				}
				if gr != nil && gr.ContentReplaced {
					result.Content = gr.ReplacementContent
				}
			}
			// Blocked outputs still contain the original material in the SDK's
			// local ToolResult; never serialize that content into this fixture.
			if !blocked {
				c.FinalContent = &result.Content
			}
		}
		encoded, err := json.Marshal(c)
		if err != nil {
			return nil, err
		}
		for _, fragment := range sensitive {
			escaped, _ := json.Marshal(fragment)
			if strings.Contains(string(encoded), string(escaped[1:len(escaped)-1])) {
				return nil, fmt.Errorf("builtin case %q would expose synthetic material", c.Name)
			}
		}
	}
	encoded, err := json.MarshalIndent(fixture, "", "  ")
	if err != nil {
		return nil, err
	}
	return append(encoded, '\n'), nil
}
