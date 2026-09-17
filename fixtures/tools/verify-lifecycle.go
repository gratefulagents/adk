// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/verify-lifecycle.go ../../target/debug/examples/lifecycle_replay
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	tools "github.com/gratefulagents/sdk/pkg/agentsdk/tools/fs"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func setup(root string) {
	must(os.Mkdir(filepath.Join(root, "dir"), 0700))
	must(os.Mkdir(filepath.Join(root, "empty"), 0700))
	must(os.WriteFile(filepath.Join(root, "a"), []byte("alpha"), 0600))
	must(os.WriteFile(filepath.Join(root, "dir/file"), []byte("beta"), 0700))
	must(os.WriteFile(filepath.Join(root, "original"), []byte("linked"), 0600))
	must(os.Link(filepath.Join(root, "original"), filepath.Join(root, "hardlink")))
	must(os.Symlink("a", filepath.Join(root, "symlink")))
}
func tree(root string) map[string]string {
	out := map[string]string{}
	must(filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		value := info.Mode().String()
		if info.Mode().IsRegular() {
			bytes, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			value += " " + string(bytes)
		}
		if info.Mode()&os.ModeSymlink != 0 {
			target, err := os.Readlink(path)
			if err != nil {
				return err
			}
			value += " " + target
		}
		out[rel] = value
		return nil
	}))
	return out
}
func main() {
	data, err := os.ReadFile("../../fixtures/tools/lifecycle.json")
	must(err)
	var cases []struct {
		Name       string          `json:"name"`
		Arguments  json.RawMessage `json:"arguments"`
		ErrorOnly  bool            `json:"error_only"`
		FullAccess bool            `json:"full_access"`
		Content    *string         `json:"content"`
	}
	must(json.Unmarshal(data, &cases))
	root, err := os.MkdirTemp("", "adk-lifecycle-parity-")
	must(err)
	defer os.RemoveAll(root)
	command := exec.Command(os.Args[1])
	command.Stderr = os.Stderr
	stdin, err := command.StdinPipe()
	must(err)
	stdout, err := command.StdoutPipe()
	must(err)
	must(command.Start())
	defer func() { _ = command.Process.Kill(); _ = command.Wait() }()
	encoder := json.NewEncoder(stdin)
	scanner := bufio.NewScanner(stdout)
	implementations := map[string]agentsdk.Tool{"Move": &tools.MoveTool{}, "Delete": &tools.DeleteTool{}, "Write": &tools.WorkspaceWriteFileTool{}, "Edit": &tools.WorkspaceEditTool{}}
	for i, test := range cases {
		goRoot := filepath.Join(root, fmt.Sprintf("go-%d", i))
		rustRoot := filepath.Join(root, fmt.Sprintf("rust-%d", i))
		must(os.Mkdir(goRoot, 0700))
		must(os.Mkdir(rustRoot, 0700))
		setup(goRoot)
		setup(rustRoot)
		if test.Content != nil {
			must(os.WriteFile(filepath.Join(goRoot, "a"), []byte(*test.Content), 0600))
			must(os.WriteFile(filepath.Join(rustRoot, "a"), []byte(*test.Content), 0600))
		}
		implementation := implementations[test.Name]
		if test.Name == "Write" && test.FullAccess {
			implementation = &tools.FileWriteTool{}
		}
		if test.Name == "Edit" && test.FullAccess {
			implementation = &tools.FileEditTool{}
		}
		expected, err := implementation.Execute(context.Background(), test.Arguments, goRoot)
		must(err)
		must(encoder.Encode(map[string]any{"name": test.Name, "arguments": test.Arguments, "work_dir": rustRoot, "full_access": test.FullAccess}))
		if !scanner.Scan() {
			panic("Rust replay stopped")
		}
		var got struct {
			Content []struct {
				Text string `json:"text"`
			} `json:"content"`
			IsError     bool `json:"is_error"`
			ShouldPause bool `json:"should_pause"`
		}
		must(json.Unmarshal(scanner.Bytes(), &got))
		if got.IsError != expected.IsError || got.ShouldPause != expected.ShouldPause || len(got.Content) != 1 || (!test.ErrorOnly && strings.ReplaceAll(got.Content[0].Text, rustRoot, "<root>") != strings.ReplaceAll(expected.Content, goRoot, "<root>")) {
			panic(fmt.Sprintf("case %d: Go=%+v Rust=%s", i, expected, scanner.Text()))
		}
		if !reflect.DeepEqual(tree(goRoot), tree(rustRoot)) {
			panic(fmt.Sprintf("case %d: filesystem mismatch: Go=%v Rust=%v", i, tree(goRoot), tree(rustRoot)))
		}
	}
	fmt.Printf("%d SDK lifecycle results and resulting filesystem trees verified\n", len(cases))
}
