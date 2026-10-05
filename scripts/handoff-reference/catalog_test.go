package main

import (
	"bytes"
	"os"
	"reflect"
	"strings"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestCatalogSemantics(t *testing.T) {
	outputs := map[string]catalogOutput{}
	for _, s := range catalogScenarios() {
		o := observeCatalog(s.Input)
		outputs[s.Name] = o
		if !reflect.DeepEqual(o.Parent.Tools, o.ReturnedTools) {
			t.Fatalf("%s: parent/returned tools differ", s.Name)
		}
		for _, h := range o.Handoffs {
			if !h.HasInputFilter || len(h.FilterOutput) != 1 || h.FilterOutput[0].Type != agentsdk.RunItemMessage || h.FilterOutput[0].Message.Text != "retained catalog history" {
				t.Fatalf("%s: unexpected attached filter result", s.Name)
			}
			if h.HasInputType || h.HasOnHandoff || h.HasEnabledPredicate || !h.Tool.ReadOnly {
				t.Fatalf("%s: unexpected handoff flags", s.Name)
			}
			if h.SpecialistKey != nil && !reflect.DeepEqual(h.Target, o.Specialists[*h.SpecialistKey]) {
				t.Fatalf("%s: target is not returned specialist", s.Name)
			}
		}
	}
	base := outputs["two_roles_order_and_access"]
	if base.Handoffs[0].ToolName != "transfer_to_zeta_review" || base.Handoffs[1].ToolName != "transfer_to_alpha_builder" {
		t.Fatal("handoff order must follow catalog, not lexical order")
	}
	if base.Handoffs[0].Description != "Review changes." || base.Handoffs[1].Description != "Transfer the conversation to the alpha-builder specialist." {
		t.Fatal("handoff descriptions")
	}
	review := base.Specialists["Zeta Review"]
	if review.Instructions != "  Review exactly.\n" || review.HandoffDescription != "  Review changes.  " {
		t.Fatal("specialist text was normalized")
	}
	wantRead := []toolView{{"z_read", true, "inert_host_tool"}, {"Bash", true, "sdk_read_only_bash"}, {"Finish", true, "inert_host_tool"}}
	if !reflect.DeepEqual(review.Tools, wantRead) {
		t.Fatalf("real adapter/filter/order: %#v", review.Tools)
	}
	build := base.Specialists["alpha-builder"]
	if len(build.Tools) != 4 || build.Tools[1].Name != "a_write" || build.Tools[2].ReadOnly {
		t.Fatal("full access or exact-case signal stripping changed")
	}
	if len(base.Parent.Tools) != 7 || base.Parent.Tools[2].ReadOnly {
		t.Fatal("parent bundle unexpectedly filtered")
	}
	if !base.Parent.DynamicInstructions || !strings.Contains(base.Parent.Instructions, "Handoffs (transfer full control):") {
		t.Fatal("parent delegation instructions missing")
	}
	if outputs["parent_tool_surface_off"].Parent.Tools != nil || len(outputs["parent_tool_surface_off"].Specialists["alpha-builder"].Tools) != 4 {
		t.Fatal("parent tool gate must not gate specialist host tools")
	}
	for _, name := range []string{"gates_both_off", "gates_handoffs_only_no_fallback", "explicit_gates_override_legacy", "subagents_generic_flag_alone_not_gate"} {
		if outputs[name].Specialists != nil || outputs[name].Handoffs != nil {
			t.Fatalf("%s: disabled gate", name)
		}
	}
	if len(outputs["gates_subagents_only"].Specialists) != 2 || outputs["gates_subagents_only"].Handoffs != nil {
		t.Fatal("subagent-only gate")
	}
	for _, name := range []string{"subagents_status_gate", "subagents_control_gate"} {
		if !reflect.DeepEqual(outputs[name], base) {
			t.Fatalf("%s: gate should enable same definitions", name)
		}
	}
	if outputs["empty_catalog_no_fallback"].Specialists == nil || len(outputs["empty_catalog_no_fallback"].Specialists) != 0 {
		t.Fatal("enabled empty catalog should return nonnil empty map")
	}
	for _, name := range []string{"gates_handoffs_only_generic", "empty_catalog_handoff_fallback", "empty_catalog_both_fallbacks"} {
		h := outputs[name].Handoffs
		if len(h) != 1 || h[0].SpecialistKey != nil || h[0].Target.Name != "specialist" || h[0].Target.FallbackModels != nil || h[0].Target.Tools != nil {
			t.Fatalf("%s: generic handoff must be independent, with no tools or fallbacks", name)
		}
	}
	if len(outputs["empty_catalog_both_fallbacks"].Specialists) != 1 || outputs["empty_catalog_both_fallbacks"].Specialists["agent"].Name != "agent" || outputs["empty_catalog_subagent_fallback"].Handoffs != nil {
		t.Fatal("generic specialist and generic handoff are distinct")
	}
	edge := outputs["sanitizer_unicode_and_punctuation"].Handoffs
	if edge[0].ToolName != "transfer_to_quipe_____x_9" || edge[1].ToolName != "transfer_to_specialist" {
		t.Fatalf("sanitizer observation: %s, %s", edge[0].ToolName, edge[1].ToolName)
	}
	for _, name := range []string{"duplicate_trimmed_role_first_wins", "blank_role_skipped"} {
		if !reflect.DeepEqual(outputs[name], base) {
			t.Fatalf("%s: should skip role", name)
		}
	}
	collision := outputs["sanitized_handoff_collision"].Handoffs
	if collision[0].ToolName != collision[1].ToolName || *collision[0].SpecialistKey == *collision[1].SpecialistKey {
		t.Fatal("collision must preserve distinct targets")
	}
	for name, model := range map[string]string{"role_model_and_fallback_override": "role-model", "mode_defaults_override_role": "mode-model", "mode_role_routing_wins": "routed-model", "mode_routing_gate_off": "role-model"} {
		if outputs[name].Specialists["Zeta Review"].Model != model || outputs[name].Parent.Model != "base-model" {
			t.Fatalf("%s: specialist precedence or parent model changed", name)
		}
	}
	for name, fallbacks := range map[string][]string{
		"role_model_and_fallback_override":          {"role-fallback-z", "role-fallback-a"},
		"mode_defaults_override_role":               {"mode-fallback-z", "mode-fallback-a"},
		"mode_role_routing_wins":                    {"routed-z", "routed-a"},
		"empty_role_fallback_inherits":              {"base-fallback-z", "base-fallback-a"},
		"empty_mode_fallback_inherits_role":         {"role-fallback"},
		"empty_routing_role_fallback_inherits_mode": {"mode-fallback-z", "mode-fallback-a"},
	} {
		if !reflect.DeepEqual(outputs[name].Specialists["Zeta Review"].FallbackModels, fallbacks) {
			t.Fatalf("%s: fallback nonempty override/order", name)
		}
	}
	routed := outputs["mode_role_routing_wins"].Specialists["Zeta Review"].Settings
	if routed.ReasoningEffort != "minimal" || routed.ThinkingBudget != 1024 || routed.TextVerbosity != "low" || routed.MaxTokens != 1234 {
		t.Fatalf("routed settings: %#v", routed)
	}
	settings := outputs["settings_config_pointer_not_applied"].Parent.Settings
	if settings.MaxTokens != 1234 || settings.ToolChoice != "" || settings.ReasoningEffort != "low" || settings.ParallelToolCalls == nil || *settings.ParallelToolCalls {
		t.Fatalf("builder settings scope: %#v", settings)
	}
	access := outputs["role_access_blank_and_unknown"]
	if !reflect.DeepEqual(access.Specialists["Zeta Review"].Tools, build.Tools) || !reflect.DeepEqual(access.Specialists["alpha-builder"].Tools, wantRead) {
		t.Fatal("blank access is full; unknown access is read-only")
	}
	aliases := outputs["role_access_aliases"]
	if !reflect.DeepEqual(aliases.Specialists["Zeta Review"].Tools, wantRead) || !reflect.DeepEqual(aliases.Specialists["alpha-builder"].Tools, wantRead) {
		t.Fatal("read-only aliases")
	}
	if outputs["blank_description_fallback"].Handoffs[0].Description != "Transfer the conversation to the Zeta Review specialist." {
		t.Fatal("blank description fallback")
	}
}

func TestCatalogFixtureRepeatability(t *testing.T) {
	want, err := os.ReadFile("../../fixtures/handoff/sdk-catalog-handoffs.json")
	if err != nil {
		t.Fatal(err)
	}
	for range 2 {
		got, err := generateCatalog()
		if err != nil {
			t.Fatal(err)
		}
		if !bytes.Equal(got, want) {
			t.Fatal("catalog fixture differs from Go regeneration")
		}
	}
}
