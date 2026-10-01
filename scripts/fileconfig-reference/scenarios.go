package main

import sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"

func scenarios() []testCase {
	var cases []testCase
	add := func(name string, files map[string]string, active string, queries ...query) {
		if files == nil {
			files = map[string]string{}
		}
		cases = append(cases, testCase{Name: name, SourceOnly: []string{}, Input: input{Files: files, ActiveMode: active, RootStyle: "literal", Queries: queries}})
	}
	q := func(op string) query { return query{Operation: op} }
	get := func(name string) query { return query{Operation: "GetMode", Lookup: name} }
	modeQueries := []query{q("ListModes"), get("custom"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"), q("RoleCatalog")}
	sourceOnly := func(reasons ...string) { cases[len(cases)-1].SourceOnly = reasons }
	add("missing-directories", nil, "", q("dirs"), q("ListModes"), q("RoleCatalog"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"), get("unknown"))
	add("builtin-chat", nil, "chat", get("chat"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"))
	add("builtin-plan", nil, " \tplan\n", get(" PLAN "), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"))
	add("plain-full-schema", map[string]string{"modes/custom.yaml": `name: " custom "
version: " v2 "
displayName: " Display Custom "
description: " Description  with  gaps "
category: " direct "
autonomous: true
toolAccess: " analysis "
instructions: " \nFirst  line\n  Indented line\n "
modelRouting:
  defaultModel: " provider/model "
  fallbackModels: [" fallback/a ", "", "  ", "fallback/a", "fallback/b"]
  reasoningLevel: " high "
  textVerbosity: " low "
  roleOverrides:
    " planner ":
      model: " provider/planner "
      fallbackModels: [" child/a ", " ", "child/a"]
      reasoningLevel: " xhigh "
      textVerbosity: " medium "
    empty: {}
constraints:
  maxTurns: 21
  subAgentMaxTurns: 8
  maxConcurrentSubAgents: 3
  maxRetries: 4
  maxRuntimeMinutes: 12
`, "agents/planner.md": "Plan carefully.\n"}, "custom", modeQueries...)
	add("crd-spec-precedes-top-level", map[string]string{"modes/custom.yaml": `apiVersion: agents.example/v1
kind: ModeTemplate
metadata:
  name: metadata-name
name: ignored-top-level
instructions: ignored-top-level
spec:
  name: custom
  displayName: CRD Custom
  instructions: Use the spec.
  constraints:
    maxTurns: 19
    subAgentMaxTurns: 7
    maxConcurrentSubAgents: 2
    maxRetries: 5
    maxRuntimeMinutes: 13
`}, "custom", modeQueries...)
	add("metadata-name-fallback", map[string]string{"modes/custom.yaml": "metadata:\n  name: ' metadata-name '\nspec:\n  instructions: from-spec\n"}, "custom", get("custom"), get("metadata-name"), q("ModeSnapshot"))
	add("filename-name-version-fallback", map[string]string{"modes/custom.yaml": "name: '  '\nversion: ' '\ninstructions: from-file\n"}, "custom", get("custom"), q("ModeSnapshot"))
	add("zero-spec-falls-back-to-plain", map[string]string{"modes/custom.yaml": "name: plain-name\ninstructions: plain\nspec: {}\n"}, "custom", get("custom"))
	add("filename-versus-declared-name", map[string]string{"modes/custom.yaml": "name: declared-name\ndisplayName: Friendly Display\ninstructions: yes\n"}, "custom", get("custom"), get("declared-name"), get("DECLARED-NAME"), get("friendly display"), get("  FRIENDLY DISPLAY  "))
	add("yaml-before-yml", map[string]string{"modes/custom.yaml": "name: yaml-name\n", "modes/custom.yml": "name: yml-name\n"}, "custom", get("custom"), q("ListModes"))
	add("malformed-yaml-does-not-fall-through-to-yml", map[string]string{"modes/custom.yaml": "name: [\n", "modes/custom.yml": "name: good\n"}, "custom", get("custom"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"))
	add("direct-lookup-isolates-unrelated-malformed-mode", map[string]string{"modes/custom.yaml": "name: custom\ndisplayName: Friendly\n", "modes/broken.yaml": "name: [\n"}, "custom", get("custom"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"), get("CUSTOM"), get("Friendly"), get("plan"), q("ListModes"))
	add("malformed-role-isolated-from-mode", map[string]string{"modes/custom.yaml": "name: custom\n", "agents/bad.md": "---\nname: [\n---\nbody\n"}, "custom", modeQueries...)
	add("malformed-mode-isolated-from-roles", map[string]string{"modes/custom.yaml": "name: [\n", "agents/good.md": "role body\n"}, "custom", modeQueries...)
	add("mode-type-error", map[string]string{"modes/custom.yaml": "constraints:\n  maxTurns: nope\n"}, "custom", get("custom"), q("ListModes"))
	add("builtin-override-by-declared-name", map[string]string{"modes/custom.yaml": "name: CHAT\ndisplayName: Replacement\ninstructions: custom-chat\n"}, "chat", q("ListModes"), get("chat"), get("custom"), q("ModeSnapshot"))
	add("extensions-and-nested-files", map[string]string{"modes/custom.YAML": "name: custom\n", "modes/ignored.txt": "bad: [\n", "modes/nested/bad.yaml": "bad: [\n", "agents/UPPER.MD": "upper body\n", "agents/ignore.txt": "", "agents/nested/bad.md": ""}, "custom", modeQueries...)
	add("blank-traversal-and-absent-names", nil, "", get(""), get(" \t\n"), get("."), get(".."), get("../secret"), get("a/b"), get("a\\b"), get("/absolute"), get("unknown"))
	var cancelled []query
	for _, item := range []query{q("ListModes"), q("RoleCatalog"), get("chat"), get(""), get("../secret"), q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective")} {
		item.Cancelled = true
		cancelled = append(cancelled, item)
	}
	add("cancelled-with-active-mode", nil, "plan", cancelled...)
	add("cancelled-without-active-mode", nil, " \t", cancelled...)
	add("unknown-active-mode", nil, "missing", q("ModeSnapshot"), q("PermissionMode"), q("ModeDirective"))
	add("directory-read-errors", map[string]string{"modes": "not a directory", "agents": "not a directory"}, "custom", modeQueries...)
	add("role-frontmatter-alias-precedence", map[string]string{"agents/custom.md": `---
name: " declared-role "
description: "  preserved description  "
tool_access: " analysis "
toolAccess: full
model_override: " provider/override "
model: provider/ignored
---
  Role body  with  gaps.
    next line
`, "agents/fallback.md": `---
name: " "
tool_access: " "
toolAccess: execution
model_override: " "
model: " provider/fallback "
---
Fallback body
`, "agents/plain.md": " \n Plain body\n "}, "", q("RoleCatalog"))
	add("role-empty-instructions", map[string]string{"agents/custom.md": "---\nname: custom\n---\n \n"}, "", q("RoleCatalog"))
	add("role-unclosed-frontmatter", map[string]string{"agents/custom.md": "---\nname: custom\nbody without closing delimiter\n"}, "", q("RoleCatalog"))
	sourceOnly("unterminated-frontmatter-policy")
	add("role-frontmatter-crlf", map[string]string{"agents/custom.md": "---\r\nname: custom\r\ntool_access: readonly\r\n---\r\n body\r\n"}, "", q("RoleCatalog"))
	add("duplicate-declared-mode-names", map[string]string{"modes/a.yaml": "name: custom\ninstructions: first\n", "modes/b.yaml": "name: custom\ninstructions: second\n"}, "custom", q("ListModes"), get("custom"))
	sourceOnly("duplicate-name-policy")
	add("duplicate-declared-role-names", map[string]string{"agents/a.md": "---\nname: custom\n---\nfirst\n", "agents/b.md": "---\nname: custom\n---\nsecond\n"}, "", q("RoleCatalog"))
	sourceOnly("duplicate-name-policy")
	add("unknown-yaml-fields", map[string]string{"modes/custom.yaml": "name: custom\nunknownField: ignored\nconstraints:\n  maxTurns: 3\n  unknownField: ignored\n", "agents/custom.md": "---\nunknownField: ignored\n---\nbody\n"}, "custom", modeQueries...)
	sourceOnly("unknown-field-policy")
	add("duplicate-yaml-key-rejection", map[string]string{"modes/custom.yaml": "name: first\nname: second\n"}, "custom", get("custom"))
	add("zero-and-negative-constraints", map[string]string{"modes/custom.yaml": "constraints:\n  maxTurns: 0\n  subAgentMaxTurns: -2\n  maxConcurrentSubAgents: 0\n  maxRetries: -4\n  maxRuntimeMinutes: 0\n"}, "custom", get("custom"), q("ModeSnapshot"))
	sourceOnly("zero-negative-limit-policy")
	add("empty-routing-and-constraints", map[string]string{"modes/custom.yaml": "modelRouting: {}\nconstraints: {}\n"}, "custom", get("custom"))
	for _, access := range []string{"", "read-only", "read_only", "readonly", "analysis", "full", "execution", "write", "workspace-write", " MYSTERY "} {
		name := access
		if name == "" {
			name = "inherit"
		}
		add("access-"+name, map[string]string{"modes/custom.yaml": "toolAccess: '" + access + "'\n", "agents/custom.md": "---\ntool_access: '" + access + "'\n---\nbody\n"}, "custom", get("custom"), q("PermissionMode"), q("ModeDirective"), q("RoleCatalog"))
		if access == " MYSTERY " {
			sourceOnly("unknown-access-policy")
		}
	}
	var formatQueries []query
	formatQueries = append(formatQueries, query{Operation: "BuildModeDirective"}, query{Operation: "BuildModeDirective", Template: &sdkmode.TemplateSpec{}})
	for _, access := range []string{"read-only", "read_only", "readonly", "analysis", " ANALYSIS ", "full", "mystery"} {
		formatQueries = append(formatQueries, query{Operation: "BuildModeDirective", Template: &sdkmode.TemplateSpec{Name: " fallback ", DisplayName: " Display ", Description: " \nDescription  exact\n ", ToolAccess: access, Instructions: " \nBody\n  indent\n "}})
	}
	formatQueries = append(formatQueries, query{Operation: "BuildModeDirective", Template: &sdkmode.TemplateSpec{Name: " fallback ", DisplayName: " \t", Instructions: "only body"}})
	add("direct-formatter", nil, "", formatQueries...)
	for _, style := range []string{"padded", "default", "tilde", "tilde-child"} {
		add("dirs-"+style, nil, "", q("dirs"))
		cases[len(cases)-1].Input.RootStyle = style
	}
	for _, style := range []string{"default", "tilde", "tilde-child"} {
		add("dirs-home-unset-"+style, nil, "", q("dirs"))
		cases[len(cases)-1].Input.RootStyle = style
		cases[len(cases)-1].Input.HomeUnset = true
		sourceOnly("home-policy")
	}
	return cases
}
