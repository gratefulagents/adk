// SPDX-License-Identifier: GPL-3.0-only
// SDK v0.0.115, 1dc92b73900fac74dc357a938e4b5eee6392b418.
// Run from repos/sdk: go run ../../fixtures/tools/state-memory-plan-generate.go > ../../fixtures/tools/state-memory-plan-expected.json
package main

import (
	"context"
	"encoding/json"
	"github.com/google/uuid"
	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	mem "github.com/gratefulagents/sdk/pkg/agentsdk/memory"
	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
	mt "github.com/gratefulagents/sdk/pkg/agentsdk/tools/memory"
	tools "github.com/gratefulagents/sdk/pkg/agentsdk/tools/projectstate"
	"github.com/gratefulagents/sdk/pkg/agentsdk/tools/signal"
	"os"
	"strings"
	"time"
)

type step struct {
	Name    string `json:"name"`
	Input   any    `json:"input"`
	Format  string `json:"format"`
	Save    string `json:"save,omitempty"`
	Output  any    `json:"output"`
	IsError bool   `json:"is_error"`
}

func replay(steps []step, catalog []sdk.Tool) []step {
	byName := map[string]sdk.Tool{}
	for _, t := range catalog {
		byName[t.Name()] = t
	}
	ids, reverse := map[string]string{}, map[string]string{}
	for i := range steps {
		st := &steps[i]
		raw, _ := json.Marshal(st.Input)
		var input any
		json.Unmarshal(raw, &input)
		input = normalize(input, ids)
		raw, _ = json.Marshal(input)
		result, err := byName[st.Name].Execute(context.Background(), raw, "call")
		if err != nil {
			panic(err)
		}
		if result.ShouldPause {
			panic("unexpected pause")
		}
		st.IsError = result.IsError
		var output any
		st.Format = "json"
		if json.Unmarshal([]byte(result.Content), &output) != nil {
			st.Format = "text"
			output = result.Content
		}
		if st.Save != "" {
			id := output.(map[string]any)["id"].(string)
			ids[st.Save] = id
			reverse[id] = st.Save
		}
		st.Output = normalize(output, reverse)
	}
	return steps
}

type artifactStore struct {
	content *string
	summary string
}

func (s *artifactStore) UpsertArtifact(_ context.Context, session uuid.UUID, kind, content, url, hash string, metadata json.RawMessage) (any, error) {
	if session != uuid.Nil || kind != "plan" {
		panic("artifact identity")
	}
	var m map[string]string
	json.Unmarshal(metadata, &m)
	s.summary = m["summary"]
	s.content = &content
	return nil, nil
}
func (s *artifactStore) GetArtifact(_ context.Context, session uuid.UUID, kind string) (*signal.Artifact, error) {
	if session != uuid.Nil || kind != "plan" {
		panic("artifact identity")
	}
	if s.content == nil {
		return nil, nil
	}
	return &signal.Artifact{Content: *s.content}, nil
}
func main() {
	dir, err := os.MkdirTemp("", "state-gap-fixture")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(dir)
	store, err := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: dir, ProjectID: "tool-contract"})
	if err != nil {
		panic(err)
	}
	raw, err := os.ReadFile("../../fixtures/tools/state-memory-plan-cases.json")
	if err != nil {
		panic(err)
	}
	var steps []step
	if err := json.Unmarshal(raw, &steps); err != nil {
		panic(err)
	}
	state := replay(steps, tools.Tools(store, " agent "))
	memorySteps := []step{
		{Name: "Memory", Input: map[string]any{"action": "store", "content": "nullable", "tags": nil, "limit": nil, "id": nil}, Save: "$m"},
		{Name: "Memory", Input: map[string]any{"action": "store", "content": "null tag", "tags": []any{nil, "keep"}}, Save: "$n"},
		{Name: "Memory", Input: map[string]any{"action": "list", "tags": []any{nil}, "limit": 1}},
		{Name: "Memory", Input: map[string]any{"action": "search", "content": "tag", "tags": []any{nil, "keep"}, "limit": 1}},
		{Name: "Memory", Input: map[string]any{"action": "delete", "id": " \t$m\n"}},
		{Name: "Memory", Input: map[string]any{"action": "list", "tags": nil, "content": nil, "limit": nil}},
	}
	memory := replay(memorySteps, []sdk.Tool{mt.New(mem.NewInMemoryStore(), "trusted", "source", "")})
	plans := []any{}
	for _, content := range []*string{nil, new(string)} {
		s := &artifactStore{content: content}
		t := signal.PlanTools(s, uuid.Nil)[1]
		r, e := t.Execute(context.Background(), json.RawMessage(`{}`), "")
		if e != nil {
			panic(e)
		}
		plans = append(plans, map[string]any{"name": "get_plan", "stored": content, "input": map[string]any{}, "text": r.Content, "is_error": r.IsError})
	}
	for _, plan := range []string{"short", strings.Repeat("a", 199), strings.Repeat("a", 200), strings.Repeat("é", 100), strings.Repeat("€", 70), strings.Repeat("a", 201)} {
		s := &artifactStore{}
		t := signal.PlanTools(s, uuid.Nil)[0]
		input := map[string]any{"plan": plan, "summary": nil}
		raw, _ := json.Marshal(input)
		r, e := t.Execute(context.Background(), raw, "")
		if e != nil {
			panic(e)
		}
		plans = append(plans, map[string]any{"name": "save_plan", "input": input, "text": r.Content, "is_error": r.IsError, "summary": s.summary})
	}
	missing := []any{}
	for _, t := range []sdk.Tool{mt.New(nil, "ns", "", ""), &signal.SavePlanTool{}, &signal.GetPlanTool{}} {
		r, e := t.Execute(context.Background(), json.RawMessage(`{"action":"list","plan":"x"}`), "")
		if e != nil {
			panic(e)
		}
		missing = append(missing, map[string]any{"name": t.Name(), "text": r.Content, "is_error": r.IsError})
	}
	out := map[string]any{"state": state, "memory": memory, "plans": plans, "missing_store": missing}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
func normalize(value any, ids map[string]string) any {
	switch v := value.(type) {
	case map[string]any:
		for key, item := range v {
			v[key] = normalize(item, ids)
		}
	case []any:
		for i, item := range v {
			v[i] = normalize(item, ids)
		}
	case string:
		if id := ids[v]; id != "" {
			return id
		}
		if _, err := time.Parse(time.RFC3339Nano, v); err == nil {
			return "<time>"
		}
		if strings.HasPrefix(v, "comment_") {
			return "<comment>"
		}
		for id, alias := range ids {
			v = strings.ReplaceAll(v, id, alias)
		}
		return v
	}
	return value
}
