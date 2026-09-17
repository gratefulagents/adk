// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/project-state/tools.go > ../../fixtures/project-state/tools.json
package main

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"time"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
	tools "github.com/gratefulagents/sdk/pkg/agentsdk/tools/projectstate"
)

type step struct {
	Name    string         `json:"name"`
	Input   map[string]any `json:"input"`
	Save    string         `json:"save,omitempty"`
	Output  any            `json:"output"`
	IsError bool           `json:"is_error"`
}

func main() {
	dir, err := os.MkdirTemp("", "state-tool-fixture")
	if err != nil {
		panic(err)
	}
	defer os.RemoveAll(dir)
	store, err := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: dir, ProjectID: "tool-contract"})
	if err != nil {
		panic(err)
	}
	catalog := tools.Tools(store, "agent")
	definitions := []any{}
	byName := map[string]sdk.Tool{}
	for _, tool := range catalog {
		byName[tool.Name()] = tool
		definitions = append(definitions, map[string]any{"name": tool.Name(), "description": tool.Description(), "input_schema": json.RawMessage(tool.InputSchema()), "read_only": tool.IsReadOnly(), "requires_approval": tool.NeedsApproval()})
	}
	steps := []step{
		{Name: "task_create", Input: map[string]any{"title": "blocker"}, Save: "$a"},
		{Name: "task_create", Input: map[string]any{"title": "dependent", "priority": 0}, Save: "$b"},
		{Name: "task_link", Input: map[string]any{"id": "$b", "depends_on": "$a"}},
		{Name: "task_ready", Input: map[string]any{}},
		{Name: "task_claim", Input: map[string]any{"id": "$a"}},
		{Name: "task_comment", Input: map[string]any{"id": "$a", "body": "note"}},
		{Name: "task_update", Input: map[string]any{"id": "$a", "labels": []string{"label"}}},
		{Name: "task_update", Input: map[string]any{"id": "$a", "labels": []string{}}},
		{Name: "task_close", Input: map[string]any{"id": "$a", "reason": "done"}},
		{Name: "task_ready", Input: map[string]any{}},
		{Name: "task_link", Input: map[string]any{"id": "$b", "depends_on": "$a", "action": "remove"}},
		{Name: "task_show", Input: map[string]any{"id": "$a"}},
		{Name: "memory_remember", Input: map[string]any{"content": "durable engineering preference", "kind": "semantic", "scope": "user", "tags": []string{"style"}}, Save: "$m"},
		{Name: "memory_update", Input: map[string]any{"id": "$m", "content": "compact engineering preference", "tags": []string{}}},
		{Name: "memory_list", Input: map[string]any{"kinds": []string{"semantic"}}},
		{Name: "memory_recall", Input: map[string]any{"query": "engineering"}},
		{Name: "memory_stats", Input: map[string]any{}},
		{Name: "memory_delete", Input: map[string]any{"id": "$m"}},
		{Name: "memory_stats", Input: map[string]any{}},
	}
	ids := map[string]string{}
	reverse := map[string]string{}
	for i := range steps {
		st := &steps[i]
		input := map[string]any{}
		for key, value := range st.Input {
			if s, ok := value.(string); ok && ids[s] != "" {
				value = ids[s]
			}
			input[key] = value
		}
		raw, _ := json.Marshal(input)
		result, err := byName[st.Name].Execute(context.Background(), raw, "call")
		if err != nil {
			panic(err)
		}
		st.IsError = result.IsError
		var output any
		if err := json.Unmarshal([]byte(result.Content), &output); err != nil {
			panic(err)
		}
		if st.Save != "" {
			id := output.(map[string]any)["id"].(string)
			ids[st.Save] = id
			reverse[id] = st.Save
		}
		st.Output = normalize(output, reverse)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(map[string]any{"definitions": definitions, "steps": steps}); err != nil {
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
	}
	return value
}
