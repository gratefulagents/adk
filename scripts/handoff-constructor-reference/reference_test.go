// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"encoding/json"
	"os"
	"testing"
)

func TestNativeHandoffConstructorReference(t *testing.T) {
	cases := []map[string]any{}
	for _, name := range []string{"Code Reviewer", "a   b", "a..b", "a__b", "a--b", "__-A--_", " ", "", "中文", "A中文B", "İSTANBUL", "a_??_b", "a-\u00a0-b", "\ufeffx"} {
		for _, description := range []string{"", "custom description", " \n"} {
			target := &Agent{Name: name, HandoffDescription: description}
			h := NewHandoff(target)
			tool := h.ToTool()
			cases = append(cases, map[string]any{"name": name, "target_description": description, "tool_name": tool.Name(), "description": tool.Description(), "schema": json.RawMessage(tool.InputSchema()), "read_only": tool.IsReadOnly(), "approval": tool.NeedsApproval(), "same_target": h.Agent == target, "has_filter": h.InputFilter != nil, "overrides": false})
		}
	}
	for _, description := range []string{"override", ""} {
		target := &Agent{Name: "Target", HandoffDescription: "original"}
		h := NewHandoff(target, WithToolName("custom_transfer"), WithDescription(description))
		tool := h.ToTool()
		cases = append(cases, map[string]any{"name": target.Name, "target_description": target.HandoffDescription, "tool_name": tool.Name(), "description": tool.Description(), "schema": json.RawMessage(tool.InputSchema()), "read_only": tool.IsReadOnly(), "approval": tool.NeedsApproval(), "same_target": h.Agent == target, "has_filter": h.InputFilter != nil, "overrides": true})
	}
	data, err := json.MarshalIndent(map[string]any{"cases": cases}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
