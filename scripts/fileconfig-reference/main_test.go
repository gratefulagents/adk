package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

func TestOracle(t *testing.T) {
	f, err := generate()
	if err != nil {
		t.Fatal(err)
	}
	byName := map[string]testCase{}
	for _, c := range f.Cases {
		if _, exists := byName[c.Name]; exists {
			t.Fatalf("duplicate case %q", c.Name)
		}
		byName[c.Name] = c
		if len(c.Input.Queries) != len(c.Output) {
			t.Fatalf("%s: query/output mismatch", c.Name)
		}
	}
	result := func(name string, index int, target any) {
		t.Helper()
		obs := byName[name].Output[index]
		if obs.Error != nil {
			t.Fatalf("%s[%d]: %+v", name, index, obs.Error)
		}
		if err := json.Unmarshal(obs.Result, target); err != nil {
			t.Fatal(err)
		}
	}
	var modes []sdkmode.TemplateSpec
	result("missing-directories", 1, &modes)
	if len(modes) != 2 || modes[0].Name != "chat" || modes[1].Name != "plan" {
		t.Fatalf("builtins: %+v", modes)
	}
	var plain, crd sdkmode.TemplateSpec
	result("plain-full-schema", 2, &plain)
	result("crd-spec-precedes-top-level", 2, &crd)
	if plain.Constraints == nil || *plain.Constraints != (sdkmode.Constraints{MaxTurns: 21, SubAgentMaxTurns: 8, MaxConcurrentSubAgents: 3, MaxRetries: 4, MaxRuntimeMinutes: 12}) {
		t.Fatalf("plain constraints: %+v", plain.Constraints)
	}
	if crd.Constraints == nil || *crd.Constraints != (sdkmode.Constraints{MaxTurns: 19, SubAgentMaxTurns: 7, MaxConcurrentSubAgents: 2, MaxRetries: 5, MaxRuntimeMinutes: 13}) {
		t.Fatalf("CRD constraints: %+v", crd.Constraints)
	}
	if plain.ModelRouting == nil || len(plain.ModelRouting.FallbackModels) != 3 || plain.ModelRouting.FallbackModels[0] != plain.ModelRouting.FallbackModels[1] {
		t.Fatalf("routing: %+v", plain.ModelRouting)
	}
	if _, ok := plain.ModelRouting.RoleOverrides[" planner "]; !ok {
		t.Fatal("role routing key whitespace was lost")
	}
	var spec sdkmode.TemplateSpec
	result("yaml-before-yml", 0, &spec)
	if spec.Name != "yaml-name" {
		t.Fatalf("extension precedence: %+v", spec)
	}
	for i, obs := range byName["direct-lookup-isolates-unrelated-malformed-mode"].Output {
		if (i < 4) != (obs.Error == nil) {
			t.Fatalf("direct lookup isolation[%d]: %+v", i, obs)
		}
	}
	for _, name := range []string{"malformed-role-isolated-from-mode", "malformed-mode-isolated-from-roles"} {
		for i, obs := range byName[name].Output {
			wantErr := (name == "malformed-role-isolated-from-mode") == (i == 5)
			if (obs.Error != nil) != wantErr {
				t.Fatalf("%s[%d]: %+v", name, i, obs)
			}
		}
	}
	var roles agentsdk.RoleCatalog
	result("role-frontmatter-alias-precedence", 0, &roles)
	if len(roles) != 3 || roles[0].Name != "declared-role" || roles[0].ToolAccess != "read-only" || roles[0].Description != "  preserved description  " || roles[0].ModelOverride != "provider/override" || roles[1].ModelOverride != "provider/fallback" {
		t.Fatalf("roles: %+v", roles)
	}
	for i, obs := range byName["cancelled-without-active-mode"].Output {
		if i >= 5 {
			if obs.Error != nil {
				t.Fatalf("inactive cancellation[%d]: %+v", i, obs)
			}
			continue
		}
		category := "cancelled"
		if i == 3 || i == 4 {
			category = "fileconfig"
		}
		if obs.Error == nil || obs.Error.Category != category {
			t.Fatalf("cancel validation ordering[%d]: %+v", i, obs)
		}
	}
	var directive string
	for i := 2; i <= 6; i++ {
		result("direct-formatter", i, &directive)
		if !strings.Contains(directive, "Tool access: read-only.") || !strings.HasSuffix(directive, "Body\n  indent") {
			t.Fatalf("formatter[%d]: %q", i, directive)
		}
	}
	result("access- MYSTERY ", 0, &spec)
	if spec.ToolAccess != "MYSTERY" || len(byName["access- MYSTERY "].SourceOnly) == 0 {
		t.Fatal("unknown access must remain source-only and verbatim")
	}
	var permission string
	result("access- MYSTERY ", 1, &permission)
	if permission != "workspace-write" {
		t.Fatalf("Go unknown access: %q", permission)
	}
	for _, name := range []string{"duplicate-declared-mode-names", "duplicate-declared-role-names", "unknown-yaml-fields", "zero-and-negative-constraints", "role-unclosed-frontmatter", "dirs-home-unset-default"} {
		if len(byName[name].SourceOnly) == 0 {
			t.Fatalf("%s missing policy annotation", name)
		}
	}
}

func TestRegenerationIsByteIdentical(t *testing.T) {
	first, err := generate()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generate()
	if err != nil {
		t.Fatal(err)
	}
	a, err := json.Marshal(first)
	if err != nil {
		t.Fatal(err)
	}
	b, err := json.Marshal(second)
	if err != nil {
		t.Fatal(err)
	}
	if string(a) != string(b) {
		t.Fatal("regeneration differs")
	}
	if strings.Contains(string(a), "fileconfig-reference-") {
		t.Fatal("temporary path leaked")
	}
}
