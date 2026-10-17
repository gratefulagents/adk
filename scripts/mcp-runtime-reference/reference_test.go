// SPDX-License-Identifier: GPL-3.0-only
package runtime

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"testing"
	"time"

	sdkmcp "github.com/gratefulagents/sdk/pkg/agentsdk/mcp"
	"github.com/gratefulagents/sdk/pkg/agentsdk/policy"
	"github.com/gratefulagents/sdk/pkg/agentsdk/sandbox"
)

type runtimeMCPCase struct {
	Name       string   `json:"name"`
	Enabled    bool     `json:"enabled"`
	AllServers bool     `json:"all_servers"`
	Servers    []string `json:"servers"`
	AllTools   bool     `json:"all_tools"`
	Tools      []string `json:"tools"`
	Resources  bool     `json:"resources"`
}
type runtimeMCPTool struct {
	Name   string `json:"name"`
	Server string `json:"server"`
	Raw    string `json:"raw"`
}
type runtimeMCPObservation struct {
	Name    string           `json:"name"`
	Servers []string         `json:"servers"`
	Tools   []string         `json:"tools"`
	Catalog []runtimeMCPTool `json:"catalog"`
}

func TestRuntimeMCPReference(t *testing.T) {
	data, err := os.ReadFile(os.Getenv("MCP_RUNTIME_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []runtimeMCPCase
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	observations := []runtimeMCPObservation{}
	for _, c := range cases {
		dir := t.TempDir()
		config := sdkmcp.Config{MCPServers: map[string]sdkmcp.ServerConfig{}}
		for _, name := range []string{"chosen", "other"} {
			config.MCPServers[name] = sdkmcp.ServerConfig{Command: "/usr/bin/python3", Args: []string{"-u", os.Getenv("MCP_RUNTIME_PEER"), filepath.Join(dir, name), "ok"}, TrustReadOnlyHint: true}
		}
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		bundle, err := BuildToolBundle(ctx, Config{WorkDir: dir, PermissionMode: policy.PermissionModeDangerFullAccess, CommandSandboxConfig: &sandbox.Config{Mode: "disabled"}, MCPConfig: &config, Features: &Features{MCP: MCPFeatures{Enabled: c.Enabled, AllowAllServers: c.AllServers, AllowedServers: c.Servers, AllowAllTools: c.AllTools, AllowedTools: c.Tools, ResourceTools: c.Resources}}})
		if err != nil {
			cancel()
			t.Fatalf("%s: %v", c.Name, err)
		}
		o := runtimeMCPObservation{Name: c.Name, Servers: []string{}, Tools: []string{}, Catalog: []runtimeMCPTool{}}
		o.Servers = append(o.Servers, bundle.MCPServers...)
		for _, tool := range bundle.Tools {
			o.Tools = append(o.Tools, tool.Name())
			if dynamic, ok := tool.(*sdkmcp.DynamicTool); ok {
				o.Catalog = append(o.Catalog, runtimeMCPTool{Name: tool.Name(), Server: dynamic.Descriptor.ServerName, Raw: dynamic.Descriptor.ToolName})
			}
		}
		sort.Strings(o.Servers)
		sort.Strings(o.Tools)
		sort.Slice(o.Catalog, func(i, j int) bool { return o.Catalog[i].Name < o.Catalog[j].Name })
		for _, closer := range bundle.Closers {
			if err := closer.Close(); err != nil {
				t.Fatal(err)
			}
		}
		cancel()
		observations = append(observations, o)
	}
	data, err = json.MarshalIndent(observations, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("MCP_RUNTIME_OUTPUT"), append(data, '\n'), 0600); err != nil {
		t.Fatal(err)
	}
}
