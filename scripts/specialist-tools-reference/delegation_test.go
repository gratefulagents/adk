// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

func nativeDelegationGuides() []map[string]any {
	type handoff struct{ Tool, Target, Description string }
	cases := []struct {
		Name        string
		Specialists map[string]string
		Handoffs    []handoff
	}{
		{Name: "empty", Specialists: map[string]string{}},
		{Name: "blank-keys", Specialists: map[string]string{"": "ignored", " \t\u0085": "ignored"}},
		{Name: "original-guidance", Specialists: map[string]string{"executor": "Implement a bounded change"}},
		{Name: "sorted-raw-keys-and-descriptions", Specialists: map[string]string{"z": "", " a ": " \n ", "a": "line one\nline two", "界": "Unicode", "Á": "accent", "": "skip"}},
		{Name: "handoffs-only", Specialists: map[string]string{}, Handoffs: []handoff{{"transfer_to_z", "z", ""}, {"transfer_to_a", "a", "custom description"}, {"", " target ", ""}}},
		{Name: "combined", Specialists: map[string]string{"worker": ""}, Handoffs: []handoff{{"same", "target", " "}, {"same", "next", "second\nline"}}},
	}
	out := []map[string]any{}
	for _, c := range cases {
		specialists := map[string]*Agent{}
		for key, description := range c.Specialists {
			specialists[key] = &Agent{Name: "agent_" + key, HandoffDescription: description}
		}
		agent := &Agent{}
		for _, h := range c.Handoffs {
			agent.Handoffs = append(agent.Handoffs, &Handoff{ToolName: h.Tool, Description: h.Description, Agent: &Agent{Name: h.Target}})
		}
		out = append(out, map[string]any{"name": c.Name, "specialists": c.Specialists, "handoffs": c.Handoffs, "guide": BuildDelegationGuide(agent, specialists)})
	}
	return out
}
