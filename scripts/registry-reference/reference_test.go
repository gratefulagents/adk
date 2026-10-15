// SPDX-License-Identifier: GPL-3.0-only
package tools

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"

	"github.com/gratefulagents/sdk/pkg/agentsdk/policy"
)

type registryInput struct {
	ID                 string   `json:"id"`
	Permission         string   `json:"permission"`
	RemoteWrites       bool     `json:"remote_writes"`
	DefaultConstructor bool     `json:"default_constructor"`
	Signals            bool     `json:"signals"`
	Browser            bool     `json:"browser"`
	WithoutWeb         bool     `json:"without_web"`
	PrivateURLs        bool     `json:"private_urls"`
	AsyncShell         bool     `json:"async_shell"`
	Think              bool     `json:"think"`
	Terminal           bool     `json:"terminal"`
	AllowMutating      []string `json:"allow_mutating"`
}
type registryTool struct {
	Name       string `json:"name"`
	SourceType string `json:"source_type"`
	Contract   string `json:"contract"`
}
type registryObservation struct {
	Input   registryInput   `json:"input"`
	Names   []string        `json:"names"`
	Tools   []registryTool  `json:"tools"`
	Missing bool            `json:"missing"`
	Options map[string]bool `json:"options"`
}

// Calls the actual pinned constructor and reflects its actual registered objects.
// No tool executes, Chromium launches, or Vision directory integration is exercised.
func TestNativeRegistrySelectionReference(t *testing.T) {
	for _, key := range []string{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS", "GRATEFUL_BASH_MAX_TIMEOUT_MS", "GRATEFUL_BASH_MAX_OUTPUT_BYTES"} {
		t.Setenv(key, "")
	}
	profiles := []registryInput{
		{ID: "baseline"},
		{ID: "signals", Signals: true},
		{ID: "browser_public", Browser: true},
		{ID: "browser_private", Browser: true, PrivateURLs: true},
		{ID: "without_web", WithoutWeb: true},
		{ID: "private_urls", PrivateURLs: true},
		{ID: "async_shell", AsyncShell: true},
		{ID: "think", Think: true},
		{ID: "terminal", Terminal: true},
		{ID: "all_options", Signals: true, Browser: true, PrivateURLs: true, AsyncShell: true, Think: true, Terminal: true},
		{ID: "all_without_web", Signals: true, Browser: true, PrivateURLs: true, AsyncShell: true, Think: true, Terminal: true, WithoutWeb: true},
		{ID: "allowlist_baseline", AllowMutating: []string{"Write", "Edit", "ApplyPatch", "Move", "Delete", "Terminal", "BashStart", "BashPoll", "BashKill", "missing_tool"}},
		{ID: "allowlist_options", Browser: true, AsyncShell: true, Terminal: true, AllowMutating: []string{"Write", "Edit", "ApplyPatch", "Move", "Delete", "Browser", "Terminal", "BashStart", "BashPoll", "BashKill", "missing_tool"}},
	}
	cases := []registryInput{{ID: "default_constructor", Permission: string(policy.PermissionModeWorkspaceWrite), RemoteWrites: true, DefaultConstructor: true}}
	for _, profile := range profiles {
		for _, mode := range []policy.PermissionMode{policy.PermissionModeReadOnly, policy.PermissionModeWorkspaceWrite, policy.PermissionModeDangerFullAccess} {
			for _, remote := range []bool{false, true} {
				input := profile
				input.ID = fmt.Sprintf("%s/%s/remote_%t", profile.ID, mode, remote)
				input.Permission = string(mode)
				input.RemoteWrites = remote
				cases = append(cases, input)
			}
		}
	}
	contracts := map[string]json.RawMessage{}
	observations := []registryObservation{}
	for _, input := range cases {
		t.Run(input.ID, func(t *testing.T) {
			if input.AllowMutating == nil {
				input.AllowMutating = []string{}
			}
			opts := []RegistryOption{}
			if !input.DefaultConstructor {
				remote := policy.GitRemoteWritesDisabled
				if input.RemoteWrites {
					remote = policy.GitRemoteWritesEnabled
				}
				opts = append(opts, WithPermissionMode(policy.PermissionMode(input.Permission)), WithGitRemoteWrites(remote), WithPrivateNetworkURLs(input.PrivateURLs))
			}
			if input.Signals {
				opts = append(opts, WithSignalTools())
			}
			if input.Browser {
				opts = append(opts, WithBrowserTools())
			}
			if input.WithoutWeb {
				opts = append(opts, WithoutWebTools())
			}
			if input.AsyncShell {
				opts = append(opts, WithAsyncShellTools())
			}
			if input.Think {
				opts = append(opts, WithThinkTool())
			}
			if input.Terminal {
				opts = append(opts, WithInteractiveTerminal())
			}
			if len(input.AllowMutating) > 0 {
				opts = append(opts, WithAllowedMutatingTools(input.AllowMutating...))
			}
			registry := NewRegistry(t.TempDir(), opts...)
			defer func() {
				for _, closer := range registry.Closers() {
					if err := closer.Close(); err != nil {
						t.Error(err)
					}
				}
			}()
			observation := registryObservation{Input: input, Names: registry.Names(), Tools: []registryTool{}, Missing: registry.Get("missing_tool") == nil}
			observation.Options = map[string]bool{
				"signals": registry.signals, "browser": registry.browser,
				"without_web": registry.disableWeb, "private_urls": registry.allowPrivateNetworkURLs,
				"async_shell": registry.asyncShell, "think": registry.thinkTool,
				"terminal": registry.interactiveTerminal,
			}
			toolNames := []string{}
			for _, tool := range registry.Tools() {
				typ := reflect.TypeOf(tool)
				for typ.Kind() == reflect.Pointer {
					typ = typ.Elem()
				}
				sourceType := strings.TrimPrefix(typ.PkgPath(), "github.com/gratefulagents/sdk/") + "." + typ.Name()
				contract, err := json.Marshal(map[string]any{
					"name": tool.Name(), "description": tool.Description(), "input_schema": tool.InputSchema(),
					"read_only": tool.IsReadOnly(), "requires_approval": tool.NeedsApproval(),
				})
				if err != nil {
					t.Fatal(err)
				}
				key := fmt.Sprintf("%s-%x", tool.Name(), sha256.Sum256(contract))
				contracts[key] = contract
				observation.Tools = append(observation.Tools, registryTool{tool.Name(), sourceType, key})
				toolNames = append(toolNames, tool.Name())
				if registry.Get(tool.Name()) != tool {
					t.Fatalf("Get(%q) did not return registered object", tool.Name())
				}
			}
			if !sort.StringsAreSorted(observation.Names) || !reflect.DeepEqual(observation.Names, toolNames) || !observation.Missing {
				t.Fatalf("registry enumeration/lookup invariant failed: %+v", observation)
			}
			observations = append(observations, observation)
		})
	}
	output, err := json.Marshal(map[string]any{"cases": observations, "contracts": contracts})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("REGISTRY_OUTPUT"), output, 0600); err != nil {
		t.Fatal(err)
	}
}
