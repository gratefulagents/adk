// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/verify-search.go /absolute/path/to/search_replay
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/search"
)

func main() {
	data, err := os.ReadFile("../../fixtures/tools/search.json")
	must(err)
	var fixture struct {
		Files map[string]string `json:"files"`
		Cases []struct {
			Name      string         `json:"name"`
			Arguments map[string]any `json:"arguments"`
			Paginate  bool           `json:"paginate"`
			ErrorOnly bool           `json:"error_only"`
		} `json:"cases"`
	}
	must(json.Unmarshal(data, &fixture))
	root, err := os.MkdirTemp("", "adk-search-parity-")
	must(err)
	defer os.RemoveAll(root)
	for path, content := range fixture.Files {
		path = filepath.Join(root, path)
		must(os.MkdirAll(filepath.Dir(path), 0700))
		must(os.WriteFile(path, []byte(content), 0600))
	}
	must(os.WriteFile(filepath.Join(root, "large.txt"), []byte(strings.Repeat("x", 100100)), 0600))
	must(os.WriteFile(filepath.Join(root, "long-line.txt"), []byte("needle before\n"+strings.Repeat("x", 2<<20)+"\nneedle after\n"), 0600))
	outside, err := os.CreateTemp("", "adk-search-secret-")
	must(err)
	defer os.Remove(outside.Name())
	_, err = outside.WriteString("needle secret outside\n")
	must(err)
	must(outside.Close())
	must(os.Link(outside.Name(), filepath.Join(root, "hardlink.txt")))
	must(os.Symlink(outside.Name(), filepath.Join(root, "escape.txt")))
	tools := map[string]agentsdk.Tool{"read_file": &search.ReadFileTool{}, "list_files": &search.ListFilesTool{}, "glob": &search.GlobTool{}, "grep": &search.GrepTool{}}
	command := exec.Command(os.Args[1])
	command.Stderr = os.Stderr
	stdin, err := command.StdinPipe()
	must(err)
	stdout, err := command.StdoutPipe()
	must(err)
	must(command.Start())
	defer func() { _ = command.Process.Kill(); _ = command.Wait() }()
	scanner := bufio.NewScanner(stdout)
	scanner.Buffer(make([]byte, 64*1024), 20<<20)
	encoder := json.NewEncoder(stdin)
	calls := 0
	for index, test := range fixture.Cases {
		for page := 0; page < 100; page++ {
			raw, err := json.Marshal(test.Arguments)
			must(err)
			expected, goErr := tools[test.Name].Execute(context.Background(), raw, root)
			must(encoder.Encode(map[string]any{"name": test.Name, "work_dir": root, "arguments": test.Arguments}))
			if !scanner.Scan() {
				panic(fmt.Sprintf("Rust process stopped: %v", scanner.Err()))
			}
			var actual struct {
				Content []struct {
					Text string `json:"text"`
				} `json:"content"`
				IsError     bool   `json:"is_error"`
				ShouldPause bool   `json:"should_pause"`
				Error       string `json:"error"`
			}
			must(json.Unmarshal(scanner.Bytes(), &actual))
			calls++
			if (goErr != nil) != (actual.Error != "") {
				panic(fmt.Sprintf("case %d %s error mismatch: Go=%v Rust=%s", index, test.Name, goErr, actual.Error))
			}
			if goErr != nil {
				if !test.ErrorOnly {
					panic(goErr)
				}
				break
			}
			if test.ErrorOnly && expected.IsError && actual.IsError {
				break
			}
			if len(actual.Content) != 1 || actual.Content[0].Text != expected.Content || actual.IsError != expected.IsError || actual.ShouldPause != expected.ShouldPause {
				panic(fmt.Sprintf("case %d %s %s page%d differs\nGo: %s (error=%v)\nRust: %+v", index, test.Name, raw, page, expected.Content, expected.IsError, actual))
			}
			if !test.Paginate {
				break
			}
			var envelope struct {
				Truncated bool   `json:"truncated"`
				Next      string `json:"next_cursor"`
			}
			must(json.Unmarshal([]byte(expected.Content), &envelope))
			if !envelope.Truncated {
				break
			}
			if envelope.Next == "" || page == 99 {
				panic("non-terminating pagination")
			}
			test.Arguments["cursor"] = envelope.Next
		}
	}
	must(stdin.Close())
	must(command.Wait())
	fmt.Printf("%d SDK search differential calls verified (%d cases, including pagination)\n", calls, len(fixture.Cases))
}
func must(err error) {
	if err != nil {
		panic(err)
	}
}
