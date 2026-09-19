// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/schema-generate.go
// No Rust manifest is read. Definitions come from actual SDK bundle instances.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"reflect"
	"sort"
	"strconv"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/policy"
	runtime "github.com/gratefulagents/sdk/pkg/agentsdk/runtime"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/memory"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/signal"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/skills"
)

const pin = "1dc92b73900fac74dc357a938e4b5eee6392b418"

func check(err error) {
	if err != nil {
		panic(err)
	}
}

type definition struct {
	Name             string          `json:"name"`
	Description      string          `json:"description"`
	Parameters       json.RawMessage `json:"parameters"`
	ReadOnly         bool            `json:"read_only"`
	RequiresApproval bool            `json:"requires_approval"`
	ControlFlow      bool            `json:"control_flow"`
	Timeout          int             `json:"timeout_seconds"`
}
type sample struct {
	Access      string            `json:"access"`
	Environment map[string]string `json:"environment"`
	Definitions []definition      `json:"definitions"`
}

// Tool has no exported ControlFlow method. Read the pinned registry's actual
// exemption switch with Go's parser, rather than duplicating it or Rust metadata.
// This is registry control flow, not runner's broader user-interaction allowlist.
func registryControlFlow() map[string]bool {
	file, err := parser.ParseFile(token.NewFileSet(), "pkg/agentsdk/tools/registry.go", nil, 0)
	check(err)
	names := map[string]bool{}
	for _, decl := range file.Decls {
		fn, ok := decl.(*ast.FuncDecl)
		if !ok || fn.Name.Name != "isRegistryControlFlowTool" {
			continue
		}
		ast.Inspect(fn.Body, func(n ast.Node) bool {
			clause, ok := n.(*ast.CaseClause)
			if !ok {
				return true
			}
			for _, expr := range clause.List {
				lit, ok := expr.(*ast.BasicLit)
				if !ok || lit.Kind != token.STRING {
					panic("unexpected control-flow switch")
				}
				name, err := strconv.Unquote(lit.Value)
				check(err)
				names[name] = true
			}
			return true
		})
	}
	if len(names) != 4 {
		panic("SDK registry control-flow switch changed")
	}
	return names
}
func enable(v reflect.Value) {
	for i := 0; i < v.NumField(); i++ {
		field := v.Field(i)
		switch field.Kind() {
		case reflect.Bool:
			field.SetBool(true)
		case reflect.Struct:
			enable(field)
		default:
			panic("unexpected feature type")
		}
	}
}
func main() {
	commit, err := exec.Command("git", "rev-parse", "HEAD").Output()
	check(err)
	if strings.TrimSpace(string(commit)) != pin {
		panic("SDK checkout does not match pin")
	}
	control := registryControlFlow()
	work, err := os.MkdirTemp("", "adk-schema-")
	check(err)
	defer os.RemoveAll(work)
	catalog := skills.NewRegistryFromEntries(nil)
	installer := skills.NewInstaller(catalog)
	// No stores/services are invoked: these SDK concrete types expose definitions
	// without executing tools. Extras still traverse the real runtime bundle API.
	extras := []agentsdk.Tool{memory.New(nil, "schema", "", ""), &signal.SavePlanTool{}, &signal.GetPlanTool{}, &skills.SearchTool{Registry: catalog}, &skills.InstallTool{Installer: installer, WorkDir: work}, &skills.ListInstalledTool{Installer: installer, WorkDir: work}}
	features := &runtime.Features{}
	enable(reflect.ValueOf(&features.Tools).Elem())
	features.ProjectState = runtime.ProjectStateFeatures{TaskTools: true, MemoryTools: true, PrimeTool: true}
	rows := []sample{}
	for _, env := range []map[string]string{{}, {"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS": "999", "GRATEFUL_BASH_MAX_TIMEOUT_MS": "500"}, {"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS": "240000", "GRATEFUL_BASH_MAX_TIMEOUT_MS": "120000"}, {"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS": "bad", "GRATEFUL_BASH_MAX_TIMEOUT_MS": "-1"}} {
		for _, key := range []string{"GRATEFUL_BASH_DEFAULT_TIMEOUT_MS", "GRATEFUL_BASH_MAX_TIMEOUT_MS", "GRATEFUL_BASH_MAX_OUTPUT_BYTES"} {
			check(os.Unsetenv(key))
		}
		for key, value := range env {
			check(os.Setenv(key, value))
		}
		for _, mode := range []struct {
			label string
			value policy.PermissionMode
		}{{"read_only", policy.PermissionModeReadOnly}, {"workspace_write", policy.PermissionModeWorkspaceWrite}, {"full_access", policy.PermissionModeDangerFullAccess}} {
			cfg := runtime.Config{WorkDir: work, ProjectStateDir: work + "/state", ProjectID: "schema-fixture", PermissionMode: mode.value, Features: features, ExtraTools: extras, AllowPrivateNetworkURLs: true, GitRemoteWrites: policy.GitRemoteWritesEnabled}
			bundle, err := runtime.BuildToolBundle(context.Background(), cfg)
			check(err)
			row := sample{Access: mode.label, Environment: env, Definitions: []definition{}}
			for _, tool := range bundle.Tools {
				row.Definitions = append(row.Definitions, definition{tool.Name(), tool.Description(), tool.InputSchema(), tool.IsReadOnly(), tool.NeedsApproval(), control[tool.Name()], tool.TimeoutSeconds()})
			}
			sort.Slice(row.Definitions, func(i, j int) bool { return row.Definitions[i].Name < row.Definitions[j].Name })
			for _, closer := range bundle.Closers {
				check(closer.Close())
			}
			rows = append(rows, row)
		}
	}
	fixture := struct {
		SDKCommit         string   `json:"sdk_commit"`
		ControlFlowSource string   `json:"control_flow_source"`
		Samples           []sample `json:"samples"`
	}{pin, "pkg/agentsdk/tools/registry.go:isRegistryControlFlowTool (AST; SDK Tool has no ControlFlow method)", rows}
	encoded, err := json.MarshalIndent(fixture, "", "  ")
	check(err)
	check(os.WriteFile("../../fixtures/tools/schema-definitions.json", append(encoded, '\n'), 0644))
	fmt.Printf("%d SDK bundle schema samples generated\n", len(rows))
}
