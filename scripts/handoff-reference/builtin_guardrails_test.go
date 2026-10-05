package main

import (
	"bytes"
	"encoding/json"
	"os"
	"reflect"
	"strings"
	"testing"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/guardrails"
)

func TestBuiltinGuardrailsFixture(t *testing.T) {
	first, err := generateBuiltinGuardrails()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generateBuiltinGuardrails()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatal("builtin guardrail generation is not deterministic")
	}
	golden, err := os.ReadFile("../../fixtures/handoff/sdk-builtin-guardrails.json")
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, golden) {
		t.Fatal("builtin guardrail fixture is stale")
	}
	var fixture builtinFixture
	if err := json.Unmarshal(first, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.SchemaVersion != 1 || fixture.SDKRevision != "1dc92b73900fac74dc357a938e4b5eee6392b418" {
		t.Fatal("unexpected fixture version")
	}
	if !reflect.DeepEqual(fixture.InputGuardNames, []string{"block-destructive-commands", "detect-secret-leak"}) || !reflect.DeepEqual(fixture.OutputGuardNames, []string{"detect-secret-in-output"}) {
		t.Fatal("builtin guard order changed")
	}
	if len(fixture.Cases) != 87 {
		t.Fatalf("got %d cases, want 87", len(fixture.Cases))
	}
	seen := map[string]bool{}
	for _, c := range fixture.Cases {
		t.Run(c.Name, func(t *testing.T) {
			if seen[c.Name] {
				t.Fatal("duplicate case")
			}
			seen[c.Name] = true
			names := fixture.InputGuardNames
			if c.Phase == "output" {
				names = fixture.OutputGuardNames
			}
			if len(c.Guards) == 0 || len(c.Guards) > len(names) {
				t.Fatal("missing or extra guard observations")
			}
			for i, g := range c.Guards {
				if g.Name != names[i] || g.Error != "" || g.Result == nil {
					t.Fatal("unexpected guard name, error, or nil result")
				}
				if g.Result.TripwireTriggered && i != len(c.Guards)-1 {
					t.Fatal("continued after tripwire")
				}
			}
			last := c.Guards[len(c.Guards)-1].Result
			if !last.TripwireTriggered && len(c.Guards) != len(names) {
				t.Fatal("skipped guard without tripwire")
			}
			if c.Phase == "input" && c.FinalContent != nil {
				t.Fatal("input leaked final content")
			}
			if c.Recipe.Kind != "literal" {
				if c.Params != nil || c.Content != nil {
					t.Fatal("recipe case includes raw material")
				}
				content, fragments, err := builtinRecipeContent(c.Recipe)
				if err != nil {
					t.Fatal(err)
				}
				again, _, err := builtinRecipeContent(c.Recipe)
				if err != nil || content != again {
					t.Fatal("recipe is not deterministic")
				}
				for _, fragment := range fragments {
					escaped, _ := json.Marshal(fragment)
					if bytes.Contains(first, escaped[1:len(escaped)-1]) {
						t.Fatal("fixture contains synthetic material")
					}
				}
				if c.Phase == "input" {
					wantTrip := c.Name != "input_gcp_escaped_json_text"
					if last.TripwireTriggered != wantTrip {
						t.Fatal("unexpected synthetic input detection")
					}
				} else {
					hardBlock := strings.Contains(c.Recipe.Kind, "aws") || c.Recipe.Kind == "gcp_service_account"
					if last.TripwireTriggered != hardBlock || last.ContentReplaced == hardBlock {
						t.Fatal("unexpected hard-block/redaction behavior")
					}
					if hardBlock {
						if c.FinalContent != nil || last.ReplacementContent != "" {
							t.Fatal("blocked content persisted")
						}
					} else {
						if c.FinalContent == nil || *c.FinalContent != last.ReplacementContent {
							t.Fatal("replacement not propagated")
						}
						if !strings.HasPrefix(*c.FinalContent, "before\n") {
							t.Fatal("lost safe prefix")
						}
						if c.Recipe.Kind != "pem_unterminated" && !strings.Contains(*c.FinalContent, "\nafter\n\n") {
							t.Fatal("lost safe suffix")
						}
						if c.Recipe.Kind == "pem_unterminated" && strings.Contains(*c.FinalContent, "after") {
							t.Fatal("unterminated key did not redact to end")
						}
					}
				}
			} else if c.Phase == "output" {
				if last.TripwireTriggered || last.ContentReplaced || c.FinalContent == nil || *c.FinalContent != *c.Content {
					t.Fatal("ordinary output changed")
				}
			}
		})
	}
	for _, g := range guardrails.BuiltinToolOutputGuardrails() {
		result, err := g.Fn(nil, nil, &sdk.FunctionTool{ToolName: "read_file"}, sdk.ToolResult{Content: string(first)})
		if err != nil || result == nil || result.TripwireTriggered || result.ContentReplaced {
			t.Fatal("fixture itself triggers SDK secret detection")
		}
	}
}

func TestBuiltinGuardrailsInputMatrix(t *testing.T) {
	data, err := generateBuiltinGuardrails()
	if err != nil {
		t.Fatal(err)
	}
	var fixture builtinFixture
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	byName := map[string]builtinCase{}
	for _, c := range fixture.Cases {
		byName[c.Name] = c
	}
	check := func(name string, want bool) {
		t.Helper()
		c, ok := byName[name]
		if !ok || len(c.Guards) == 0 {
			t.Fatalf("missing case %s", name)
		}
		if c.Guards[0].Result.TripwireTriggered != want {
			t.Fatalf("%s: destructive guard tripwire differs", name)
		}
	}
	for _, tool := range []string{"bash", "BASH", "custom_BaSh_tool", "shell", "ShElL", "exec", "ExEcUtE", "read_file", "sh"} {
		shellLike := tool != "read_file" && tool != "sh"
		check("name_"+tool+"_safe", false)
		check("name_"+tool+"_destructive", shellLike)
		check("name_"+tool+"_malformed", shellLike)
	}
	for _, name := range []string{"cmd_only", "command_wins_blocked", "empty_command_fallback", "null_command_fallback", "array", "string", "number", "empty", "trailing", "command_number", "command_bool", "command_array", "command_object", "cmd_number", "valid_command_wrong_cmd"} {
		check("params_"+name, true)
	}
	for _, name := range []string{"command_wins_safe", "empty_object", "empty_fields", "unknown_field", "null"} {
		check("params_"+name, false)
	}
}
