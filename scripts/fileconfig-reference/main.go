package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk/host/fileconfig"
	sdkmode "github.com/gratefulagents/sdk/pkg/agentsdk/mode"
)

const sdkRevision = "1dc92b73900fac74dc357a938e4b5eee6392b418"

type query struct {
	Operation string                `json:"operation"`
	Lookup    string                `json:"lookup"`
	Cancelled bool                  `json:"cancelled"`
	Template  *sdkmode.TemplateSpec `json:"template"`
}
type input struct {
	Files      map[string]string `json:"files"`
	ActiveMode string            `json:"active_mode"`
	RootStyle  string            `json:"root_style"`
	HomeUnset  bool              `json:"home_unset"`
	Queries    []query           `json:"queries"`
}
type observation struct {
	Result json.RawMessage `json:"result"`
	Error  *failure        `json:"error"`
}
type failure struct {
	Message  string `json:"message"`
	Category string `json:"category"`
}
type testCase struct {
	Name       string        `json:"name"`
	SourceOnly []string      `json:"source_only"`
	Input      input         `json:"input"`
	Output     []observation `json:"output"`
}
type fixture struct {
	SchemaVersion int        `json:"schema_version"`
	SDKRevision   string     `json:"sdk_revision"`
	Cases         []testCase `json:"cases"`
}

func execute(in input) ([]observation, error) {
	root, err := os.MkdirTemp("", "fileconfig-reference-")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(root)
	home, hadHome := os.LookupEnv("HOME")
	defer func() {
		if hadHome {
			_ = os.Setenv("HOME", home)
		} else {
			_ = os.Unsetenv("HOME")
		}
	}()
	if in.HomeUnset {
		err = os.Unsetenv("HOME")
	} else {
		err = os.Setenv("HOME", filepath.Join(root, "home"))
	}
	if err != nil {
		return nil, err
	}
	paths := make([]string, 0, len(in.Files))
	for name := range in.Files {
		paths = append(paths, name)
	}
	sort.Strings(paths)
	for _, name := range paths {
		path := filepath.Join(root, name)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return nil, err
		}
		if err := os.WriteFile(path, []byte(in.Files[name]), 0o644); err != nil {
			return nil, err
		}
	}
	rootArg := root
	switch in.RootStyle {
	case "", "literal":
	case "padded":
		rootArg = " \t" + root + " \n"
	case "default":
		rootArg = " \t"
	case "tilde":
		rootArg = "~"
	case "tilde-child":
		rootArg = "~/config"
	default:
		return nil, fmt.Errorf("unknown root style %q", in.RootStyle)
	}
	src := fileconfig.New(rootArg, " \twork-dir\n", fileconfig.WithActiveMode(in.ActiveMode))
	output := make([]observation, 0, len(in.Queries))
	for _, q := range in.Queries {
		ctx := context.Background()
		if q.Cancelled {
			cancelled, cancel := context.WithCancel(ctx)
			cancel()
			ctx = cancelled
		}
		var result any
		var callErr error
		switch q.Operation {
		case "BuiltinModes":
			result = fileconfig.BuiltinModes()
		case "GuardrailRules":
			result, callErr = src.GuardrailRules(ctx)
		case "HandoffHistory":
			result, callErr = src.HandoffHistory(ctx)
		case "ListModes":
			result, callErr = src.ListModes(ctx)
		case "GetMode":
			result, callErr = src.GetMode(ctx, q.Lookup)
		case "RoleCatalog":
			result, callErr = src.RoleCatalog(ctx)
		case "PermissionMode":
			result, callErr = src.PermissionMode(ctx)
		case "ModeSnapshot":
			result, callErr = src.ModeSnapshot(ctx)
		case "ModeDirective":
			result, callErr = src.ModeDirective(ctx)
		case "dirs":
			result = map[string]string{"RootDir": src.RootDir(), "ModeDir": src.ModeDir(), "AgentDir": src.AgentDir(), "DefaultRootDir": fileconfig.DefaultRootDir()}
		case "BuildModeDirective":
			result = fileconfig.BuildModeDirective(q.Template)
		default:
			return nil, fmt.Errorf("unknown operation %q", q.Operation)
		}
		raw, err := json.Marshal(result)
		if err != nil {
			return nil, err
		}
		encodedRoot, err := json.Marshal(root)
		if err != nil {
			return nil, err
		}
		obs := observation{Result: json.RawMessage(strings.ReplaceAll(string(raw), string(encodedRoot[1:len(encodedRoot)-1]), "<root>"))}
		if callErr != nil {
			category := "fileconfig"
			if errors.Is(callErr, context.Canceled) {
				category = "cancelled"
			}
			obs.Error = &failure{Message: strings.ReplaceAll(callErr.Error(), root, "<root>"), Category: category}
		}
		output = append(output, obs)
	}
	return output, nil
}

func generate() (fixture, error) {
	out := fixture{SchemaVersion: 1, SDKRevision: sdkRevision, Cases: scenarios()}
	for i := range out.Cases {
		result, err := execute(out.Cases[i].Input)
		if err != nil {
			return fixture{}, fmt.Errorf("%s: %w", out.Cases[i].Name, err)
		}
		out.Cases[i].Output = result
	}
	return out, nil
}
func main() {
	out, err := generate()
	if err == nil {
		encoder := json.NewEncoder(os.Stdout)
		encoder.SetIndent("", "  ")
		err = encoder.Encode(out)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
