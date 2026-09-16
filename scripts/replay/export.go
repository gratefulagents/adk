// SDK test-vector derivatives: GPL-3.0; see fixtures/NOTICE.md and licenses/.
// Run from repos/sdk: go run -mod=readonly ../../scripts/replay/export.go
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"

	sdk "github.com/gratefulagents/sdk/pkg/agentsdk"
	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
)

type Case struct {
	Name      string `json:"name"`
	Operation string `json:"operation"`
	Input     any    `json:"input"`
	Expected  any    `json:"expected"`
}

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	cases := []Case{}
	// llm_snapshot_test.go: TestBuildLLMResponseSnapshotCapturesReasoningAndRaw.
	items := []sdk.RunItem{
		{Type: sdk.RunItemReasoning, Reasoning: &sdk.ReasoningData{ID: "rs_1", Text: "provider-visible thinking", Signature: "sig_1", EncryptedContent: "encrypted_1"}},
		{Type: sdk.RunItemToolCall, ToolCall: &sdk.ToolCallData{ID: "call_1", Name: "Read", Input: json.RawMessage(`{"path":"README.md"}`)}},
		{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "final answer", Phase: "commentary"}},
	}
	cases = append(cases, Case{"model-items", "snapshot_items", items, sdk.SnapshotRunItems(items)})
	keepGoing := false
	response := &sdk.ModelResponse{Items: items, Usage: sdk.Usage{InputTokens: 10, OutputTokens: 5}, EndTurn: &keepGoing, Raw: map[string]any{"id": "resp_1", "output": []any{"raw content"}}}
	cases = append(cases, Case{"model-response-explicit-false", "response_snapshot", response, sdk.BuildLLMResponseSnapshot(response)})
	response2 := &sdk.ModelResponse{Items: []sdk.RunItem{{Type: sdk.RunItemMessage, Message: &sdk.MessageOutput{Text: "done"}}}}
	cases = append(cases, Case{"model-response-omitted-end-turn", "response_snapshot", response2, sdk.BuildLLMResponseSnapshot(response2)})
	// session_event_stream_test.go: TestContentEventLineHelpers (plus start partner).
	lines := []string{
		`{"type":"tool_start","agent_name":"researcher","tool":"Bash","tool_use_id":"child_1","parent_call_id":"parent_1","input_raw":"{\"command\":\"false\"}"}`,
		`{"type":"tool_end","agent_name":"researcher","tool":"Bash","tool_use_id":"child_1","parent_call_id":"parent_1","is_error":true,"output":"exit 2\nbad","tool_duration_ms":42}`,
	}
	for i, line := range lines {
		ev, ok := sdk.ParseContentEventLine(line)
		if !ok {
			panic("parse")
		}
		child, ok := sdk.ChildToolEventFromContentEvent(ev)
		if !ok {
			panic("child")
		}
		cases = append(cases, Case{fmt.Sprintf("child-tool-%d", i), "child_event", json.RawMessage(line), child})
	}
	// Real filesystem engine, following TestFilesystemStoreTaskLifecycleAndIndexes.
	dir, err := os.MkdirTemp("", "migration-state-")
	must(err)
	defer os.RemoveAll(dir)
	store, err := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: dir, ProjectID: "test-project", WorkDir: "/fixture/repo", Actor: "tester"})
	must(err)
	ctx := context.Background()
	blocker, err := store.CreateTask(ctx, ps.CreateTaskInput{Title: "Set up schema", Priority: 1})
	must(err)
	_, err = store.CreateTask(ctx, ps.CreateTaskInput{Title: "Use schema", Priority: 2, DependsOn: []string{blocker.ID}})
	must(err)
	_, err = store.ClaimTask(ctx, blocker.ID, "tester")
	must(err)
	_, err = store.CloseTask(ctx, blocker.ID, "done")
	must(err)
	reopened, err := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: dir, ProjectID: "test-project"})
	must(err)
	ready, err := reopened.ReadyTasks(ctx, ps.TaskFilter{})
	must(err)
	data, err := os.ReadFile(dir + "/events.jsonl")
	must(err)
	var events []json.RawMessage
	scanner := bufio.NewScanner(bytes.NewReader(data))
	for scanner.Scan() {
		events = append(events, append(json.RawMessage(nil), scanner.Bytes()...))
	}
	must(scanner.Err())
	ids := []string{}
	for _, task := range ready {
		ids = append(ids, task.ID)
	}
	cases = append(cases, Case{"state-ready-after-close-and-reopen", "state_ready", events, ids})
	must(reopened.Close())
	must(store.Close())
	out := map[string]any{"schema_version": 1, "sdk_revision": "1dc92b73900fac74dc357a938e4b5eee6392b418", "cases": cases}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	must(enc.Encode(out))
}
