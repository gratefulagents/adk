// SPDX-License-Identifier: GPL-3.0-only
package policy

import (
	"encoding/json"
	"os"
	"strings"
	"testing"
	"unicode/utf8"
)

func TestNativePolicyReference(t *testing.T) {
	labels := []string{"", "read-only", "workspace-write", "danger-full-access", "enabled", "disabled", "unexpected", "READ-ONLY", "WORKSPACE-WRITE", "DANGER-FULL-ACCESS", "ENABLED", "DISABLED", "workspace_write", "full_access", "read_only", "read-only\x00"}
	spaces := []rune{'\t', '\n', '\v', '\f', '\r', ' ', 0x85, 0xa0, 0x1680, 0x2000, 0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f, 0x205f, 0x3000, 0x1c, 0x180e, 0x200b, 0xfeff, 0x200e}
	inputs := append([]string{}, labels...)
	for _, label := range labels {
		for _, space := range spaces {
			inputs = append(inputs, string(space)+label+string(space))
		}
	}
	cases := []map[string]any{}
	for _, input := range inputs {
		cases = append(cases, map[string]any{"input": input, "permission": NormalizePermissionMode(input), "git": NormalizeGitRemoteWrites(GitRemoteWrites(input)), "write": PermissionMode(input).AllowsWriteTools(), "mcp_write": PermissionMode(input).AllowsMCPTool(false), "mcp_read": PermissionMode(input).AllowsMCPTool(true), "run_policy": ToolPolicyFromPermissionMode(PermissionMode(input)), "access": ToolAccessLevelFromPermissionMode(PermissionMode(input))})
	}
	trimmed := []int{}
	for r := rune(0); r <= utf8.MaxRune; r++ {
		if r >= 0xd800 && r <= 0xdfff {
			continue
		}
		if strings.TrimSpace(string(r)) == "" {
			trimmed = append(trimmed, int(r))
		}
	}
	policies := []map[string]any{}
	lists := [][]string{nil, {}, {"writer"}, {" writer ", "writer", "UNKNOWN"}}
	for _, mode := range []string{"", "read-only", "workspace-write", "danger-full-access", "WORKSPACE-WRITE", "unexpected"} {
		for _, git := range []string{"", "enabled", "disabled", "ENABLED", "unexpected"} {
			for flags := 0; flags < 8; flags++ {
				for _, list := range lists {
					input := RuntimePolicy{PermissionMode: PermissionMode(mode), GitRemoteWrites: GitRemoteWrites(git), EnableMCP: flags&1 != 0, EnableHooks: flags&2 != 0, EnablePlugins: flags&4 != 0, AllowedAgentKinds: list, AllowedSkills: list}
					normalized := input.Normalize()
					tool := NewToolPolicy(input)
					policies = append(policies, map[string]any{"input": input, "normalized": normalized, "twice": normalized.Normalize(), "tool": tool, "write": tool.AllowsWriteTools()})
				}
			}
		}
	}
	data, err := json.Marshal(map[string]any{"cases": cases, "trimmed_scalars": trimmed, "scalar_count": utf8.MaxRune + 1 - 0x800, "policies": policies})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("POLICY_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
