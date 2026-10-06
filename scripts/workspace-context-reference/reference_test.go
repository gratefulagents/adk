// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
 "encoding/json"
 "os"
 "strings"
 "testing"

 "github.com/gratefulagents/sdk/pkg/agentsdk"
)

func TestWorkspaceContextReference(t *testing.T) {
 observations := []map[string]any{}
 for _, strict := range []bool{false, true} {
  for _, work := range []string{"", " \t ", ".", " /repo/İProject ", "relative/path"} {
   for _, access := range []agentsdk.ToolAccessLevel{agentsdk.ToolAccessLevelFull, agentsdk.ToolAccessLevelReadOnly} {
    for _, names := range [][]string{{}, {"a_read", "z_read"}, {"z_read", "a_read"}} {
     cfg := Config{WorkDir: work, ToolAccess: access, Instructions: "host instructions"}
     if strict { cfg.Features = &Features{Tools: ToolFeatures{ExtraTools: true}} } else { cfg.EnableTools = true }
     tools := []agentsdk.Tool{}
     for _, name := range names { tools = append(tools, staticTool{name: name}) }
     agent, _ := BuildAgent(cfg, nil, ToolBundle{Tools: tools})
     instructions := agent.InstructionsFn(nil, agent)
     block := runtimeWorkspaceContext(cfg.normalized(), tools, resolveFeatures(cfg))
     if block != "" && !strings.HasSuffix(instructions, block) { t.Fatal("workspace block not appended") }
     observations = append(observations, map[string]any{"strict":strict, "work_dir":work, "access":access, "tools":names, "workspace":block})
    }
   }
  }
 }
 data, err := json.MarshalIndent(observations, "", "  ")
 if err != nil { t.Fatal(err) }
 if err := os.WriteFile(os.Getenv("WORKSPACE_CONTEXT_OUTPUT"), append(data, '\n'), 0600); err != nil { t.Fatal(err) }
}
