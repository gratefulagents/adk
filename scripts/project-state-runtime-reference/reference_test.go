// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
 "context"
 "encoding/json"
 "os"
 "path/filepath"
 "sort"
 "strings"
 "testing"

 "github.com/gratefulagents/sdk/pkg/agentsdk/policy"
 "github.com/gratefulagents/sdk/pkg/agentsdk/sandbox"
)

func TestProjectStateRuntimeReference(t *testing.T) {
 observations := []map[string]any{}
 for bits := 0; bits < 16; bits++ {
  dir := t.TempDir()
  stateDir := filepath.Join(dir, "state")
  bundle, err := NewBuilder(Config{
   Provider: "openai", Model: "gpt-test", APIKey: "offline-not-used",
   WorkDir: dir, ProjectID: "offline", ProjectStateDir: stateDir,
   WorkingStateText: " existing marker ",
   PermissionMode: policy.PermissionModeDangerFullAccess,
   CommandSandboxConfig: &sandbox.Config{Mode: "disabled"},
   Features: &Features{ProjectState: ProjectStateFeatures{
    PrimeContext: bits&1 != 0, TaskTools: bits&2 != 0, MemoryTools: bits&4 != 0, PrimeTool: bits&8 != 0,
   }},
  }).Build(context.Background())
  if err != nil { t.Fatal(err) }
  names := []string{}
  for _, tool := range bundle.Agent.Tools { names = append(names, tool.Name()) }
  sort.Strings(names)
  _, statErr := os.Stat(stateDir)
  observations = append(observations, map[string]any{
   "bits": bits, "tools": names, "created": statErr == nil,
   "working_state": strings.ReplaceAll(bundle.Config.WorkingStateContext, dir, "/fixture"),
  })
  for _, closer := range bundle.Closers { if err := closer.Close(); err != nil { t.Fatal(err) } }
 }
 data, err := json.MarshalIndent(observations, "", "  ")
 if err != nil { t.Fatal(err) }
 if err := os.WriteFile(os.Getenv("PROJECT_STATE_OUTPUT"), append(data, '\n'), 0600); err != nil { t.Fatal(err) }
}
