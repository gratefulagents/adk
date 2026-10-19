// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
 "encoding/json"
 "os"
 "strings"
 "testing"

 "github.com/gratefulagents/sdk/pkg/agentsdk"
 sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
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
     observations = append(observations, map[string]any{"strict":strict, "work_dir":work, "access":access, "tools":names, "workspace":block, "instructions":instructions})
    }
   }
  }
 }
 for _, enabled := range []bool{false, true} {
  for _, mode := range []struct { active string; snapshot *sdkmode.TemplateSpec }{
   {"", nil},
   {"", &sdkmode.TemplateSpec{Name:"review", DisplayName:"Reviewed", ToolAccess:"full"}},
   {"manual", &sdkmode.TemplateSpec{ToolAccess:"full"}},
   {"", &sdkmode.TemplateSpec{Name:"named", ToolAccess:"full"}},
   {"", &sdkmode.TemplateSpec{Name:"named", DisplayName:"  ", ToolAccess:"full"}},
  } {
   cfg := Config{WorkDir:".", ToolAccess:agentsdk.ToolAccessLevelFull, Instructions:"host instructions", ActiveMode:mode.active, ModeSnapshot:mode.snapshot,
    Features:&Features{Modes:ModeFeatures{Instructions:enabled}}}
   agent, _ := BuildAgent(cfg, nil, ToolBundle{})
   var snapshot any
   if mode.snapshot != nil { snapshot = map[string]any{"name":mode.snapshot.Name, "display_name":mode.snapshot.DisplayName} }
   observations = append(observations, map[string]any{"strict":true, "work_dir":".", "access":"full", "tools":[]string{},
    "mode_instructions":enabled, "active_mode":mode.active, "mode_snapshot":snapshot,
    "workspace":runtimeWorkspaceContext(cfg.normalized(), nil, resolveFeatures(cfg)), "instructions":agent.InstructionsFn(nil, agent)})
  }
 }
 data, err := json.MarshalIndent(observations, "", "  ")
 if err != nil { t.Fatal(err) }
 if err := os.WriteFile(os.Getenv("WORKSPACE_CONTEXT_OUTPUT"), append(data, '\n'), 0600); err != nil { t.Fatal(err) }
}
