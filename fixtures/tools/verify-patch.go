// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/tools/verify-patch.go ../../target/debug/examples/patch_replay
// Add --record to refresh pinned SDK results and filesystem snapshots.
package main

import (
	"bufio"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io/fs"
	"math/rand"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"syscall"

	tools "github.com/gratefulagents/sdk/pkg/agentsdk/tools/fs"
)

type File struct {
	Content string `json:"content,omitempty"`
	Mode    uint32 `json:"mode,omitempty"`
	Kind    string `json:"kind,omitempty"`
	Target  string `json:"target,omitempty"`
}
type Snapshot struct {
	Kind   string `json:"kind"`
	Mode   uint32 `json:"mode"`
	Data   string `json:"data,omitempty"`
	Target string `json:"target,omitempty"`
}
type Expected struct {
	Content string `json:"content"`
	IsError bool   `json:"is_error"`
}
type Case struct {
	Name      string              `json:"name"`
	Arguments json.RawMessage     `json:"arguments"`
	Files     map[string]File     `json:"files"`
	ErrorOnly bool                `json:"error_only"`
	Expected  Expected            `json:"expected"`
	Tree      map[string]Snapshot `json:"tree"`
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func setup(root string, files map[string]File) {
	for name, file := range files {
		path := filepath.Join(root, name)
		must(os.MkdirAll(filepath.Dir(path), 0755))
		mode := os.FileMode(file.Mode)
		if mode == 0 {
			mode = 0644
		}
		switch file.Kind {
		case "symlink", "hardlink":
			continue
		case "directory":
			must(os.MkdirAll(path, 0755))
			continue
		case "fifo":
			must(syscall.Mkfifo(path, uint32(mode)))
		default:
			data := []byte(file.Content)
			if file.Kind == "invalid_utf8" {
				data = []byte{255}
			}
			must(os.WriteFile(path, data, mode))
			must(os.Chmod(path, mode))
		}
	}
	for name, file := range files {
		path := filepath.Join(root, name)
		if file.Kind == "symlink" {
			must(os.Symlink(file.Target, path))
		}
		if file.Kind == "hardlink" {
			must(os.Link(filepath.Join(root, file.Target), path))
		}
	}
}
func tree(root string) map[string]Snapshot {
	out := map[string]Snapshot{}
	must(filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if path == root {
			return nil
		}
		relative, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		value := Snapshot{Mode: uint32(info.Mode().Perm())}
		switch {
		case info.Mode().IsRegular():
			value.Kind = "file"
			data, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			value.Data = base64.StdEncoding.EncodeToString(data)
		case info.IsDir():
			value.Kind = "directory"
		case info.Mode()&os.ModeSymlink != 0:
			value.Kind = "symlink"
			value.Target, err = os.Readlink(path)
			if err != nil {
				return err
			}
		case info.Mode()&os.ModeNamedPipe != 0:
			value.Kind = "fifo"
		default:
			return fmt.Errorf("unexpected file kind: %s", path)
		}
		out[relative] = value
		return nil
	}))
	return out
}
func generatedCases() []Case {
	var cases []Case
	add := func(name, patch string, files map[string]File) {
		arguments, err := json.Marshal(map[string]any{"patch": patch, "dry_run": true})
		must(err)
		cases = append(cases, Case{Name: name, Arguments: arguments, Files: files})
	}
	add("patch byte limit", strings.Repeat("x", 1024*1024+1), nil)
	add("hunk line limit", "*** Begin Patch\n*** Add File: new\n"+strings.Repeat("+x\n", 16385)+"*** End Patch\n", nil)
	add("hunk count limit", "*** Begin Patch\n*** Update File: a\n"+strings.Repeat("@@\n-x\n+y\n", 257)+"*** End Patch\n", nil)
	filesPatch := "*** Begin Patch\n"
	resultPatch := "*** Begin Patch\n"
	for i := 0; i < 129; i++ {
		filesPatch += fmt.Sprintf("*** Add File: file%d\n", i)
		if i < 128 {
			resultPatch += fmt.Sprintf("*** Add File: %s/%s/%s%03d\n", strings.Repeat("x", 200), strings.Repeat("y", 200), strings.Repeat("z", 100), i)
		}
	}
	add("file count limit", filesPatch+"*** End Patch\n", nil)
	add("result byte limit", resultPatch+"*** End Patch\n", nil)
	add("diff truncation", "*** Begin Patch\n*** Add File: new\n"+strings.Repeat("+abc\n", 3000)+"*** End Patch\n", nil)
	add("UTF8 split truncation", "index "+strings.Repeat("界", 3000)+"\ndiff --git a/a b/a\nnew mode 100755\n", map[string]File{"a": {Content: "old\n"}})
	large := strings.Repeat("x", 5*1024*1024)
	add("source file limit", "*** Begin Patch\n*** Delete File: a\n*** End Patch\n", map[string]File{"a": {Content: large + "x"}})
	add("patched file limit", "--- a/a\n+++ b/a\n@@ -1,0 +2 @@\n+x\n", map[string]File{"a": {Content: large}})
	aggregate := map[string]File{}
	patch := "*** Begin Patch\n"
	for i := 0; i < 13; i++ {
		name := fmt.Sprintf("file%d", i)
		aggregate[name] = File{Content: large}
		patch += "*** Delete File: " + name + "\n"
	}
	add("aggregate source limit", patch+"*** End Patch\n", aggregate)
	for _, char := range []string{"\u200b", "\u00a0", "\u0085", "\U000e0001", "\ufffd", "\v"} {
		add("quoted control", "invalid"+char+"patch", nil)
	}
	seeds := []string{
		"--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n",
		"*** Begin Patch\n*** Update File: a\n@@\n-old\n+new\n*** End Patch\n",
		"diff --git a/a b/new\nrename from a\nrename to new\nold mode 100644\nnew mode 100755\n",
		"*** Begin Patch\n*** Add File: new\n+new\n*** Delete File: a\n*** End Patch\n",
	}
	random := rand.New(rand.NewSource(7))
	for i := 0; i < 1024; i++ {
		patch := seeds[random.Intn(len(seeds))]
		index := random.Intn(len(patch))
		switch random.Intn(4) {
		case 0:
			patch = patch[:index] + patch[index+1:]
		case 1:
			patch = patch[:index] + string(" +-/@*\\\r\n0129"[random.Intn(13)]) + patch[index:]
		case 2:
			patch = patch[:index]
		case 3:
			patch = strings.ReplaceAll(patch, "\n", "\r\n")
		}
		add(fmt.Sprintf("grammar mutation %d", i), patch, map[string]File{"a": {Content: "old\n", Mode: 0600}})
	}
	return cases
}
func main() {
	data, err := os.ReadFile("../../fixtures/tools/patch.json")
	must(err)
	var cases []Case
	must(json.Unmarshal(data, &cases))
	pinnedCount := len(cases)
	cases = append(cases, generatedCases()...)
	root, err := os.MkdirTemp("", "adk-patch-parity-")
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
	scanner.Buffer(make([]byte, 65536), 2*1024*1024)
	for i := range cases {
		test := &cases[i]
		goRoot := filepath.Join(root, fmt.Sprintf("go-%d", i))
		rustRoot := filepath.Join(root, fmt.Sprintf("rust-%d", i))
		must(os.Mkdir(goRoot, 0700))
		must(os.Mkdir(rustRoot, 0700))
		setup(goRoot, test.Files)
		setup(rustRoot, test.Files)
		expected, err := (&tools.ApplyPatchTool{}).Execute(context.Background(), test.Arguments, goRoot)
		must(err)
		must(encoder.Encode(map[string]any{"arguments": test.Arguments, "work_dir": rustRoot}))
		if !scanner.Scan() {
			panic(fmt.Sprintf("Rust replay stopped: %v", scanner.Err()))
		}
		var got struct {
			Content []struct {
				Text string `json:"text"`
			} `json:"content"`
			IsError     bool `json:"is_error"`
			ShouldPause bool `json:"should_pause"`
		}
		must(json.Unmarshal(scanner.Bytes(), &got))
		if got.IsError != expected.IsError || got.ShouldPause || len(got.Content) != 1 || (!test.ErrorOnly && strings.ReplaceAll(got.Content[0].Text, rustRoot, "<root>") != strings.ReplaceAll(expected.Content, goRoot, "<root>")) {
			panic(fmt.Sprintf("case %d %s: Go=%+v Rust=%s", i, test.Name, expected, scanner.Text()))
		}
		expectedTree := tree(goRoot)
		gotTree := tree(rustRoot)
		if !reflect.DeepEqual(expectedTree, gotTree) {
			panic(fmt.Sprintf("case %d %s: filesystem mismatch Go=%v Rust=%v", i, test.Name, expectedTree, gotTree))
		}
		test.Expected = Expected{strings.ReplaceAll(expected.Content, goRoot, "<root>"), expected.IsError}
		test.Tree = expectedTree
	}
	if len(os.Args) > 2 && os.Args[2] == "--record" {
		encoded, err := json.MarshalIndent(cases[:pinnedCount], "", "  ")
		must(err)
		must(os.WriteFile("../../fixtures/tools/patch.json", append(encoded, '\n'), 0644))
	}
	fmt.Printf("%d SDK ApplyPatch results and resulting filesystem trees verified\n", len(cases))
}
