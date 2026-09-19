// Run from repos/sdk: go run ../../fixtures/tools/git-generate.go
// Only fake command execution is used; no git/gh processes or network access.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/gratefulagents/sdk/pkg/agentsdk"
	gittools "github.com/gratefulagents/sdk/pkg/agentsdk/tools/git"
)

type Reply struct {
	Output string `json:"output"`
	Error  string `json:"error,omitempty"`
}
type Step struct {
	Program string   `json:"program"`
	Args    []string `json:"argv"`
	Cwd     string   `json:"cwd"`
	Reply
}
type Case struct {
	Name      string           `json:"name"`
	Tool      string           `json:"tool"`
	Input     json.RawMessage  `json:"input"`
	Setup     string           `json:"setup,omitempty"`
	Base      string           `json:"base,omitempty"`
	Branch    string           `json:"branch,omitempty"`
	Store     string           `json:"store,omitempty"`
	Git       map[string]Reply `json:"git,omitempty"`
	GH        map[string]Reply `json:"gh,omitempty"`
	Steps     []Step           `json:"steps"`
	Content   string           `json:"content"`
	IsError   bool             `json:"is_error"`
	Artifacts []string         `json:"artifacts"`
	Exclude   string           `json:"exclude"`
	Removed   bool             `json:"removed"`
}
type runner struct {
	c    *Case
	root string
}

func (r *runner) run(program, dir string, args ...string) (string, error) {
	key := strings.Join(args, " ")
	var reply Reply
	if program == "git" {
		reply = r.c.Git[key]
		if key == "rev-parse --abbrev-ref HEAD" {
			if _, ok := r.c.Git[key]; !ok {
				reply.Output = "agent/work\n"
			}
		}
	} else {
		reply = r.c.GH[key]
	}
	if program == "git" && len(args) > 4 && args[4] == "clone" {
		key = "clone"
		if len(args) > 10 && args[9] == "--branch" {
			key = "clone " + args[10]
		}
		reply = r.c.Git[key]
		dest := args[len(args)-1]
		must(os.MkdirAll(filepath.Join(dest, ".git"), 0755))
	}
	copied := append([]string{}, args...)
	for i := range copied {
		copied[i] = strings.ReplaceAll(copied[i], r.root, "/WORK")
	}
	r.c.Steps = append(r.c.Steps, Step{program, copied, strings.ReplaceAll(dir, r.root, "/WORK"), reply})
	if reply.Error != "" {
		return reply.Output, errors.New(reply.Error)
	}
	return reply.Output, nil
}
func (r *runner) RunGit(_ context.Context, dir string, args ...string) (string, error) {
	return r.run("git", dir, args...)
}
func (r *runner) RunGH(_ context.Context, dir string, args ...string) (string, error) {
	return r.run("gh", dir, args...)
}

type sink struct{ c *Case }

func (s *sink) RecordPullRequestURL(_ context.Context, url string) error {
	s.c.Artifacts = append(s.c.Artifacts, "pr:"+url)
	return nil
}
func (s *sink) RecordIssueURL(_ context.Context, url string) error {
	s.c.Artifacts = append(s.c.Artifacts, "issue:"+url)
	return nil
}
func must(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	data, err := os.ReadFile("../../fixtures/tools/git-cases.json")
	must(err)
	var cases []Case
	must(json.Unmarshal(data, &cases))
	for i := range cases {
		c := &cases[i]
		c.Steps = []Step{}
		c.Artifacts = []string{}
		root, err := os.MkdirTemp("", "git-fixture-")
		must(err)
		dest := filepath.Join(root, "repos", "repo")
		switch c.Setup {
		case "root_git":
			must(os.MkdirAll(filepath.Join(root, ".git"), 0755))
		case "existing", "incomplete", "working", "subrepo":
			must(os.MkdirAll(filepath.Join(dest, ".git"), 0755))
			if c.Setup == "working" {
				must(os.WriteFile(filepath.Join(dest, "notes.txt"), []byte("keep"), 0644))
			}
		case "plain":
			must(os.MkdirAll(dest, 0755))
		}
		r := &runner{c, root}
		s := &sink{c}
		var tool agentsdk.Tool
		switch c.Tool {
		case "create_pull_request":
			tool = gittools.NewCreatePullRequestTool(r, s)
		case "create_github_issue":
			tool = gittools.NewCreateIssueTool(r, s)
		default:
			tool = gittools.NewAttachRepositoryTool(r, gittools.WithAttachRepositoryDefaultBaseBranch(c.Base), gittools.WithAttachRepositoryDefaultBranchName(c.Branch), gittools.WithAttachRepositoryStoreDir(c.Store))
		}
		result, err := tool.Execute(context.Background(), c.Input, root)
		must(err)
		c.Content = strings.ReplaceAll(result.Content, root, "/WORK")
		c.IsError = result.IsError
		exclude, _ := os.ReadFile(filepath.Join(root, ".git", "info", "exclude"))
		c.Exclude = string(exclude)
		_, err = os.Stat(dest)
		c.Removed = os.IsNotExist(err)
		must(os.RemoveAll(root))
	}
	out, err := json.MarshalIndent(cases, "", "  ")
	must(err)
	must(os.WriteFile("../../fixtures/tools/git-expected.json", append(out, '\n'), 0644))
	fmt.Printf("generated %d pinned Go Git fixtures\n", len(cases))
}
