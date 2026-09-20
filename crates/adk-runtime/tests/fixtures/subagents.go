// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk using scripts/replay/subagent_export.py.
package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
	"sync"
	"time"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
)

type step struct {
	Name    string         `json:"name"`
	Tool    string         `json:"tool,omitempty"`
	Args    map[string]any `json:"args"`
	Release string         `json:"release,omitempty"`
	Started string         `json:"started,omitempty"`
	Settle  []string       `json:"settle,omitempty"`
}
type scenario struct {
	Name  string   `json:"name"`
	Gates []string `json:"gates"`
	Steps []step   `json:"steps"`
}

func invoke(name, tool, args string) step {
	var a map[string]any
	must(json.Unmarshal([]byte(args), &a))
	return step{Name: name, Tool: tool, Args: a}
}
func views() []step {
	return []step{
		invoke("summary", "subagent_status", `{}`),
		invoke("graph", "subagent_status", `{"detail":"graph"}`),
		invoke("results", "subagent_status", `{"detail":"results"}`),
		invoke("reread", "subagent_status", `{"detail":"results"}`),
		invoke("empty_wait", "subagent_wait", `{}`),
	}
}
func scenarios() []scenario {
	return []scenario{
		{Name: "single_sync", Gates: []string{}, Steps: append([]step{
			invoke("spawn", "subagent", `{"message":"single"}`),
			invoke("wait_again", "subagent_wait", `{"task_ids":["$single"],"wait_for":"any"}`),
		}, views()...)},
		{Name: "dag_sync_isolation", Gates: []string{}, Steps: append([]step{
			invoke("spawn", "subagent", `{"tool_access":"read-only","tasks":[{"key":"a","message":"root"},{"key":"b","message":"isolated","agent_name":"fast","tool_access":"full","depends_on":["a"],"include_dependency_results":false},{"key":"c","message":"dependent","agent_name":"slow","depends_on":["b"]}]}`),
		}, views()...)},
		{Name: "dependency_failure", Gates: []string{}, Steps: append([]step{
			invoke("spawn", "subagent", `{"tasks":[{"key":"a","message":"failure","agent_name":"fail"},{"key":"b","message":"blocked","depends_on":["a"]},{"key":"c","message":"survivor","agent_name":"fast","depends_on":["a"],"dependency_policy":"all_terminal"}]}`),
		}, views()...)},
		{Name: "background_wait_any", Gates: []string{"fast", "slow"}, Steps: append([]step{
			invoke("spawn_fast", "subagent", `{"message":"quick","agent_name":"fast","mode":"background"}`),
			{Name: "fast_started", Started: "fast"},
			invoke("spawn_slow", "subagent", `{"message":"long","agent_name":"slow","mode":"background"}`),
			{Name: "slow_started", Started: "slow"},
			{Name: "release_fast", Release: "fast"},
			{Name: "settle_fast", Settle: []string{"quick"}},
			invoke("wait_any", "subagent_wait", `{"task_ids":["$quick","$long"],"wait_for":"any"}`),
			{Name: "release_slow", Release: "slow"},
			{Name: "settle_slow", Settle: []string{"long"}},
			invoke("wait_all", "subagent_wait", `{"task_ids":["$quick","$long"]}`),
			invoke("wait_again", "subagent_wait", `{"task_ids":["$quick","$long"],"wait_for":"any"}`),
		}, views()...)},
		{Name: "cancel", Gates: []string{"slow"}, Steps: append([]step{
			invoke("spawn", "subagent", `{"message":"cancelled","agent_name":"slow","mode":"background"}`),
			{Name: "started", Started: "slow"},
			invoke("cancel", "subagent_control", `{"task_id":"$cancelled","action":"cancel"}`),
			{Name: "settle", Settle: []string{"cancelled"}},
			invoke("wait", "subagent_wait", `{"task_ids":["$cancelled"]}`),
		}, views()...)},
		{Name: "dag_background", Gates: []string{"worker", "fast"}, Steps: append([]step{
			invoke("spawn", "subagent", `{"mode":"background","tasks":[{"key":"a","message":"first"},{"key":"b","message":"second","agent_name":"fast","depends_on":["a"]}]}`),
			{Name: "started", Started: "worker"},
			{Name: "release_root", Release: "worker"},
			{Name: "settle_root", Settle: []string{"first"}},
			{Name: "child_started", Started: "fast"},
			{Name: "release_child", Release: "fast"},
			{Name: "settle_child", Settle: []string{"second"}},
			invoke("wait", "subagent_wait", `{}`),
		}, views()...)},
	}
}

type model struct {
	mu      sync.Mutex
	gates   map[string]chan struct{}
	started map[string]chan struct{}
	calls   map[string][]any
}

func (m *model) GetResponse(ctx context.Context, req sdk.ModelRequest) (*sdk.ModelResponse, error) {
	input, _ := json.Marshal(req.Input)
	names := []string{}
	for _, tool := range req.Tools {
		names = append(names, tool.Name())
	}
	sort.Strings(names)
	observation := map[string]any{"tools": names, "dependency_evidence": strings.Contains(string(input), "evidence:"), "parent_secret": strings.Contains(string(input), "parent-secret")}
	m.mu.Lock()
	m.calls[req.Model] = append(m.calls[req.Model], observation)
	if ch, ok := m.started[req.Model]; ok {
		select {
		case <-ch:
		default:
			close(ch)
		}
	}
	m.mu.Unlock()
	if gate := m.gates[req.Model]; gate != nil {
		select {
		case <-gate:
		case <-ctx.Done():
			return nil, ctx.Err()
		}
	}
	if req.Model == "fail" {
		return nil, errors.New("controlled failure")
	}
	return &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "evidence:" + req.Model}}}}, nil
}
func (m *model) StreamResponse(ctx context.Context, req sdk.ModelRequest) (*sdk.ModelStream, error) {
	r, err := m.GetResponse(ctx, req)
	if err != nil {
		return nil, err
	}
	events := make(chan sdk.ModelStreamEvent, 1)
	events <- sdk.ModelStreamEvent{Type: sdk.ModelStreamComplete, Response: r}
	close(events)
	done := make(chan *sdk.ModelResponse, 1)
	done <- r
	close(done)
	return sdk.NewModelStream(events, done), nil
}
func (*model) GetRetryAdvice(error) *sdk.ModelRetryAdvice { return nil }
func (*model) CalculateCost(sdk.Usage) float64            { return 0 }
func (*model) Provider() string                           { return "fixture" }
func must(err error) {
	if err != nil {
		panic(err)
	}
}
func resolve(v any, ids map[string]string) any {
	switch x := v.(type) {
	case string:
		if strings.HasPrefix(x, "$") {
			id, ok := ids[x[1:]]
			if !ok {
				panic("unknown task " + x)
			}
			return id
		}
		return x
	case []any:
		out := []any{}
		for _, item := range x {
			out = append(out, resolve(item, ids))
		}
		return out
	case map[string]any:
		out := map[string]any{}
		for k, item := range x {
			out[k] = resolve(item, ids)
		}
		return out
	default:
		return v
	}
}
func normalize(v any, ids map[string]string) any {
	switch x := v.(type) {
	case string:
		for id, label := range ids {
			x = strings.ReplaceAll(x, id, label)
		}
		return x
	case []any:
		for i := range x {
			x[i] = normalize(x[i], ids)
		}
		return x
	case map[string]any:
		for k, item := range x {
			switch k {
			case "duration", "started_at", "timestamp", "duration_ms":
				x[k] = "<time>"
			default:
				x[k] = normalize(item, ids)
			}
		}
		return x
	default:
		return v
	}
}
func main() {
	output := map[string]any{"cases": []any{}}
	for _, sc := range scenarios() {
		m := &model{gates: map[string]chan struct{}{}, started: map[string]chan struct{}{}, calls: map[string][]any{}}
		agents := map[string]*sdk.Agent{}
		for _, name := range []string{"worker", "fast", "slow", "fail"} {
			m.started[name] = make(chan struct{})
			agents[name] = &sdk.Agent{Name: name, Model: name, Tools: []sdk.Tool{&sdk.FunctionTool{ToolName: "mutate", ReadOnly: false, Fn: func(context.Context, json.RawMessage) (string, error) { panic("not called") }}}}
		}
		for _, name := range sc.Gates {
			m.gates[name] = make(chan struct{})
		}
		reg := sdk.NewSubAgentScheduler(sdk.SubAgentSchedulerConfig{Runner: sdk.NewRunnerWithModel(m), Agents: agents, ToolAccessLevel: sdk.ToolAccessLevelFull, MaxTurns: 2})
		tools := sdk.BuildSubAgentTaskTools(reg, "worker")
		if output["schemas"] == nil {
			schemas := map[string]any{}
			for _, tool := range tools {
				schemas[tool.Name()] = map[string]any{"input_schema": tool.InputSchema(), "description": tool.Description(), "read_only": tool.IsReadOnly(), "requires_approval": tool.NeedsApproval()}
			}
			output["schemas"] = schemas
		}
		observations := map[string]any{}
		ids := map[string]string{}
		ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		for _, s := range sc.Steps {
			if s.Release != "" {
				close(m.gates[s.Release])
				continue
			}
			if s.Started != "" {
				select {
				case <-m.started[s.Started]:
				case <-ctx.Done():
					panic(ctx.Err())
				}
				continue
			}
			if len(s.Settle) > 0 {
				for _, label := range s.Settle {
					_, err := reg.WaitForTask(ctx, ids[label], 10000)
					must(err)
				}
				continue
			}
			input, err := json.Marshal(resolve(s.Args, ids))
			must(err)
			var result sdk.ToolResult
			for _, tool := range tools {
				if tool.Name() == s.Tool {
					result, err = tool.Execute(ctx, input, s.Name)
					must(err)
				}
			}
			var content any
			if json.Unmarshal([]byte(result.Content), &content) != nil {
				content = result.Content
			}
			observations[s.Name] = map[string]any{"content": content, "is_error": result.IsError}
			for _, task := range reg.ListTasks() {
				ids[task.Message] = task.ID
			}
		}
		tasks := map[string]any{}
		replacements := map[string]string{}
		for _, task := range reg.ListTasks() {
			replacements[task.ID] = "task:" + task.Message
			tasks[task.Message] = map[string]any{"agent": task.AgentName, "status": task.Status, "result": task.Result, "has_error": task.Error != "", "depends_on": append([]string{}, task.DependsOn...)}
		}
		m.mu.Lock()
		calls := m.calls
		m.mu.Unlock()
		observed := map[string]any{"tools": observations, "scheduler": tasks, "model_calls": calls}
		serialized, err := json.Marshal(observed)
		must(err)
		var generic any
		must(json.Unmarshal(serialized, &generic))
		output["cases"] = append(output["cases"].([]any), map[string]any{"input": sc, "expected": normalize(generic, replacements)})
		reg.CancelAll()
		cancel()
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	must(enc.Encode(output))
	fmt.Fprintln(os.Stderr, "exported actual public agentsdk tool/scheduler observations")
}
