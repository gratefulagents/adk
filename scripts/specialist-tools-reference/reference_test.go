// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
	"unicode"
)

type nativeAccessAdapter struct {
	Tool
	calls       *[]string
	id          string
	replacement Tool
}

func (a *nativeAccessAdapter) ToolForAccess(access ToolAccessLevel) Tool {
	*a.calls = append(*a.calls, a.id+":"+string(access))
	return a.replacement
}

func TestNativeSpecialistToolsReference(t *testing.T) {
	type spec struct {
		ID          string `json:"id"`
		Name        string `json:"name"`
		ReadOnly    bool   `json:"readOnly"`
		Adapter     bool   `json:"adapter"`
		Replacement string `json:"replacement"`
	}
	specs := []spec{{ID: "read", Name: "Read", ReadOnly: true}, {ID: "edit", Name: "Edit"}, {ID: "none-ro", Name: "NoneRO", ReadOnly: true, Adapter: true}, {ID: "none-mut", Name: "NoneMut", Adapter: true}, {ID: "bash", Name: "Bash", Adapter: true, Replacement: "safe"}, {ID: "safe", Name: "Bash", ReadOnly: true}, {ID: "bad", Name: "OriginalRO", ReadOnly: true, Adapter: true, Replacement: "edit"}, {ID: "rename", Name: "original", Adapter: true, Replacement: "finish"}, {ID: "finish", Name: "finish", ReadOnly: true}, {ID: "plan", Name: "present_plan", ReadOnly: true}, {ID: "ask", Name: "AskUserQuestion", ReadOnly: true}, {ID: "case", Name: "Finish", ReadOnly: true}, {ID: "space", Name: "finish ", ReadOnly: true}, {ID: "empty", ReadOnly: true}}
	calls := []string{}
	tools := map[string]Tool{}
	for _, s := range specs {
		tools[s.ID] = &FunctionTool{ToolName: s.Name, ReadOnly: s.ReadOnly}
	}
	for _, s := range specs {
		if s.Adapter {
			tools[s.ID] = &nativeAccessAdapter{Tool: tools[s.ID], calls: &calls, id: s.ID, replacement: tools[s.Replacement]}
		}
	}
	ids := []string{"read", "edit", "none-ro", "none-mut", "bash", "bad", "rename", "read", "finish", "plan", "ask", "case", "space", "empty"}
	input := []Tool{}
	for _, id := range ids {
		input = append(input, tools[id])
	}
	project := func(values []Tool) []string {
		out := []string{}
		for _, value := range values {
			found := ""
			for id, tool := range tools {
				if value == tool {
					found = id
					break
				}
			}
			if found == "" {
				t.Fatal("unexpected tool handle")
			}
			out = append(out, found)
		}
		return out
	}
	var spaces strings.Builder
	for r := rune(0); r <= 0x10ffff; r++ {
		if unicode.IsSpace(r) {
			spaces.WriteRune(r)
		}
	}
	levels := []string{"", spaces.String(), "full", "write", "workspace-write", "workspace_write", "execution", "danger-full-access", "danger_full_access", "read-only", "read_only", "readonly", "analysis", "Readonly", " FULL ", "WORKSPACE_WRITE", " worKspace-wrİte ", "full-access", "full_access", "typo", "\x1cfull\x1f", "\ufefffull", "\u200bfull", "ſull"}
	cases := []map[string]any{}
	for _, level := range levels {
		calls = []string{}
		result := FilterToolsByAccess(input, level)
		cases = append(cases, map[string]any{"access": level, "normalized": NormalizeToolAccessLevel(ToolAccessLevel(level)), "role": ToolAccessLevelFromRole(level), "outputIDs": project(result), "adapterCalls": calls})
		if len(input) != len(ids) {
			t.Fatal("input length changed")
		}
		for i, id := range ids {
			if input[i] != tools[id] {
				t.Fatal("input handle changed")
			}
		}
	}
	strips := []map[string]any{}
	for _, names := range []map[string]bool{nil, {}, DefaultSignalToolNames(), {"finish": false, "Read": true, "": true, "Finish": true, "absent": true}} {
		strips = append(strips, map[string]any{"names": names, "outputIDs": project(StripNamedSignalTools(input, names))})
	}
	defaults := DefaultSignalToolNames()
	defaults["finish"] = false
	if !DefaultSignalToolNames()["finish"] {
		t.Fatal("defaults are shared")
	}
	result := map[string]any{"guideCases": nativeDelegationGuides(), "specs": specs, "inputIDs": ids, "accessCases": cases, "stripCases": strips, "defaultNames": DefaultSignalToolNames(), "defaultStrip": project(StripSignalTools(input))}
	data, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
