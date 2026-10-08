package main

import (
	"github.com/gratefulagents/sdk/pkg/agentsdk"
	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
	sdkruntime "github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
)

func catalogScenarios() []catalogScenario {
	base := func() catalogInput {
		return catalogInput{
			AgentName: "parent", Instructions: "Coordinate the work.", Model: "base-model",
			FallbackModels: []string{"base-fallback-z", "base-fallback-a"}, Reasoning: "low", Verbosity: "medium", MaxTokens: 1234,
			Roles: agentsdk.RoleCatalog{
				{Name: "Zeta Review", Description: "  Review changes.  ", Instructions: "  Review exactly.\n", ToolAccess: "read-only"},
				{Name: "alpha-builder", Instructions: "Build exactly.", ToolAccess: "full"},
			},
			Handoffs:    sdkruntime.HandoffFeatures{Enabled: true},
			SubAgents:   sdkruntime.SubAgentFeatures{Async: sdkruntime.AsyncSubAgentFeatures{Task: true}},
			ParentTools: true, ModeRouting: true, ParallelToolCalls: true,
			HostTools: []toolView{
				{"z_read", true, "inert_host_tool"}, {"a_write", false, "inert_host_tool"},
				{"Bash", false, "sdk_bash"}, {"finish", true, "inert_host_tool"},
				{"present_plan", true, "inert_host_tool"}, {"AskUserQuestion", true, "inert_host_tool"},
				{"Finish", true, "inert_host_tool"},
			}, MCPServers: []string{"z-local", "a-local"},
		}
	}
	var cases []catalogScenario
	add := func(name string, change func(*catalogInput), reasons ...string) {
		in := base()
		if change != nil {
			change(&in)
		}
		if reasons == nil {
			reasons = []string{}
		}
		cases = append(cases, catalogScenario{name, reasons, in})
	}
	add("two_roles_order_and_access", nil)
	add("gates_both_off", func(in *catalogInput) { in.Handoffs.Enabled = false; in.SubAgents.Async.Task = false })
	add("gates_handoffs_only_no_fallback", func(in *catalogInput) { in.SubAgents.Async.Task = false })
	add("gates_handoffs_only_generic", func(in *catalogInput) { in.SubAgents.Async.Task = false; in.Handoffs.GenericFallback = true })
	add("gates_subagents_only", func(in *catalogInput) { in.Handoffs.Enabled = false })
	add("explicit_gates_override_legacy", func(in *catalogInput) {
		in.Handoffs.Enabled = false
		in.SubAgents.Async.Task = false
		in.LegacyEnableHandoffs = true
		in.LegacyEnableSubAgents = true
	})
	add("subagents_generic_flag_alone_not_gate", func(in *catalogInput) { in.SubAgents.Async.Task = false; in.SubAgents.GenericFallback = true })
	add("subagents_status_gate", func(in *catalogInput) { in.SubAgents.Async.Task = false; in.SubAgents.Async.Status = true })
	add("subagents_control_gate", func(in *catalogInput) { in.SubAgents.Async.Task = false; in.SubAgents.Async.Control = true })
	add("parent_tool_surface_off", func(in *catalogInput) { in.ParentTools = false })
	for _, flags := range []struct {
		name              string
		handoff, subagent bool
	}{
		{"empty_catalog_no_fallback", false, false}, {"empty_catalog_handoff_fallback", true, false},
		{"empty_catalog_subagent_fallback", false, true}, {"empty_catalog_both_fallbacks", true, true},
	} {
		add(flags.name, func(in *catalogInput) {
			in.Roles = agentsdk.RoleCatalog{}
			in.Handoffs.GenericFallback = flags.handoff
			in.SubAgents.GenericFallback = flags.subagent
		})
	}
	add("sanitizer_unicode_and_punctuation", func(in *catalogInput) {
		in.Roles = agentsdk.RoleCatalog{
			{Name: "  Équipe .-- X_9!?  ", Instructions: "Unicode plus ASCII.", ToolAccess: "full"},
			{Name: "审查💡", Description: "Unicode only.", Instructions: "No ASCII name.", ToolAccess: "full"},
		}
	})
	add("duplicate_trimmed_role_first_wins", func(in *catalogInput) {
		in.Roles = append(in.Roles, agentsdk.RoleSpec{Name: " Zeta Review ", Description: "Ignored duplicate", Instructions: "Ignored", ModelOverride: "ignored-model"})
	}, "duplicate trimmed role names are skipped (first wins); native strict duplicate-role policy is outside this comparison")
	add("blank_role_skipped", func(in *catalogInput) {
		in.Roles = append(in.Roles, agentsdk.RoleSpec{Name: " \t ", Instructions: "Ignored blank"})
	}, "blank roles are silently skipped; native strict blank-role validation is outside this comparison")
	add("sanitized_handoff_collision", func(in *catalogInput) {
		in.Roles = agentsdk.RoleCatalog{{Name: "a-b", Instructions: "First", ToolAccess: "full"}, {Name: "a b", Instructions: "Second", ToolAccess: "full"}}
	}, "distinct roles produce duplicate transfer_to_a_b names; native strict collision rejection is outside this comparison")
	add("host_handoff_name_collision", func(in *catalogInput) {
		in.HostTools = append(in.HostTools, toolView{"transfer_to_zeta_review", true, "inert_host_tool"})
	}, "host tool and generated handoff names collide; this builder does not validate collisions (runner execution is excluded)")
	add("role_model_and_fallback_override", func(in *catalogInput) {
		in.Roles[0].ModelOverride = "  role-model  "
		in.Roles[0].FallbackModels = []string{"role-fallback-z", "role-fallback-a"}
	})
	mode := func() *sdkmode.TemplateSpec {
		return &sdkmode.TemplateSpec{ModelRouting: &sdkmode.ModelRouting{DefaultModel: "mode-model", FallbackModels: []string{"mode-fallback-z", "mode-fallback-a"}, ReasoningLevel: "high", TextVerbosity: "high"}}
	}
	add("mode_defaults_override_role", func(in *catalogInput) {
		in.Mode = mode()
		in.Roles[0].ModelOverride = "role-model"
		in.Roles[0].FallbackModels = []string{"role-fallback"}
	})
	add("mode_role_routing_wins", func(in *catalogInput) {
		in.Mode = mode()
		in.Roles[0].ModelOverride = "role-model"
		in.Roles[0].FallbackModels = []string{"role-fallback"}
		in.Mode.ModelRouting.RoleOverrides = map[string]sdkmode.RoleModelRouting{"Zeta Review": {Model: "routed-model", FallbackModels: []string{"routed-z", "routed-a"}, ReasoningLevel: "minimal", TextVerbosity: "low"}}
	})
	add("mode_routing_gate_off", func(in *catalogInput) {
		in.Mode = mode()
		in.ModeRouting = false
		in.Roles[0].ModelOverride = "role-model"
	})
	add("empty_role_fallback_inherits", func(in *catalogInput) { in.Roles[0].FallbackModels = []string{} }, "SDK nonempty override rule: explicit empty role fallback does not clear inherited fallbacks; native explicit-empty policy may differ")
	add("empty_mode_fallback_inherits_role", func(in *catalogInput) {
		in.Mode = mode()
		in.Mode.ModelRouting.FallbackModels = []string{}
		in.Roles[0].FallbackModels = []string{"role-fallback"}
	}, "SDK nonempty override rule: empty mode fallback does not clear the role/base fallback list")
	add("empty_routing_role_fallback_inherits_mode", func(in *catalogInput) {
		in.Mode = mode()
		in.Mode.ModelRouting.RoleOverrides = map[string]sdkmode.RoleModelRouting{"Zeta Review": {FallbackModels: []string{}}}
	}, "SDK nonempty override rule: empty routing-role fallback does not clear mode fallbacks")
	add("settings_config_pointer_not_applied", func(in *catalogInput) {
		in.ModelSettings = &agentsdk.ModelSettings{MaxTokens: 999, ReasoningEffort: "max", ToolChoice: "none"}
		in.ParallelToolCalls = false
	})
	add("role_access_blank_and_unknown", func(in *catalogInput) { in.Roles[0].ToolAccess = ""; in.Roles[1].ToolAccess = "typo" })
	add("role_access_aliases", func(in *catalogInput) { in.Roles[0].ToolAccess = "read_only"; in.Roles[1].ToolAccess = "Readonly" })
	add("blank_description_fallback", func(in *catalogInput) { in.Roles[0].Description = " \t " })
	return cases
}
