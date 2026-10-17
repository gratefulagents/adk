// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: go run ../../fixtures/project-state/records/main.go -out ../../fixtures/project-state/records.json
package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"os"
	"reflect"
	"time"

	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func object(v any) map[string]any {
	b, err := json.Marshal(v)
	must(err)
	var out map[string]any
	d := json.NewDecoder(bytes.NewReader(b))
	d.UseNumber()
	must(d.Decode(&out))
	return out
}
func main() {
	out := flag.String("out", "records.json", "output file")
	flag.Parse()
	created := time.Date(2025, 2, 3, 4, 5, 6, 123456789, time.UTC)
	updated := time.Date(2025, 2, 4, 5, 6, 7, 120000000, time.UTC)
	closed := time.Date(2025, 2, 5, 6, 7, 8, 0, time.UTC)
	raw := json.RawMessage(`{"integer":18446744073709551615,"fraction":0.12345678901234567890123456789,"nested":[true,null,{"text":"é\\n"}]}`)
	comment := ps.TaskComment{ID: "comment-proof", Actor: "reviewer", Body: "Reviewed\n✓", CreatedAt: created}
	records := map[string]any{
		"Event":          ps.Event{Seq: 9223372036854775807, EventID: "event-proof", ProjectID: "project-proof", RunID: "run-proof", Actor: "agent-proof", Time: created, Type: "task.created", Payload: raw},
		"Task":           ps.Task{ID: "task-proof", Title: "Record proof", Description: "Explicit description", Type: "bug", Status: "closed", Priority: 3, Assignee: "owner-proof", DependsOn: []string{"task-before"}, Blocks: []string{"task-after"}, Labels: []string{"proof", "codec"}, Comments: []ps.TaskComment{comment}, CreatedAt: created, UpdatedAt: updated, ClosedAt: &closed, SourceRun: "run-proof", Metadata: raw},
		"TaskComment":    comment,
		"Memory":         ps.Memory{ID: "memory-proof", Kind: "episodic", Scope: "task", Content: "Persisted record proof", Tags: []string{"proof", "codec"}, TaskIDs: []string{"task-proof"}, FilePaths: []string{"src/proof.rs"}, SourceRun: "run-proof", CreatedAt: created, UpdatedAt: updated, LastReadAt: &closed, Metadata: raw},
		"SessionSummary": ps.SessionSummary{ID: "session-proof", RunID: "run-proof", Summary: "Record codecs checked", TaskIDs: []string{"task-proof"}, CreatedAt: created, UpdatedAt: updated},
	}
	fixtures := map[string]any{}
	for name, record := range records {
		cases := map[string]any{}
		add := func(name string, input map[string]any) {
			b, err := json.Marshal(input)
			must(err)
			decoded := reflect.New(reflect.TypeOf(record)).Interface()
			must(json.Unmarshal(b, decoded))
			cases[name] = map[string]any{"input": input, "expected": object(decoded)}
		}
		add("nondefault", object(record))
		add("omitted", map[string]any{})
		nulls := object(record)
		for key := range nulls {
			nulls[key] = nil
		}
		delete(nulls, "metadata")
		add("null", nulls)
		empty := object(record)
		for _, key := range []string{"run_id", "actor", "description", "assignee", "source_run"} {
			if _, ok := empty[key]; ok {
				empty[key] = ""
			}
		}
		for _, key := range []string{"depends_on", "blocks", "labels", "comments", "tags", "task_ids", "file_paths"} {
			if _, ok := empty[key]; ok {
				empty[key] = []any{}
			}
		}
		for _, key := range []string{"closed_at", "last_read_at"} {
			if _, ok := empty[key]; ok {
				empty[key] = nil
			}
		}
		delete(empty, "metadata")
		add("empty", empty)
		for _, tc := range []struct {
			name string
			at   time.Time
		}{
			{"offset", created.In(time.FixedZone("positive", 19800))},
			{"offset_negative", updated.In(time.FixedZone("negative", -12600))},
			{"offset_integral", closed.In(time.FixedZone("positive", 19800))},
			{"offset_zero", closed},
		} {
			offset := object(record)
			stamp := tc.at.Format(time.RFC3339Nano)
			if tc.name == "offset_zero" {
				stamp = tc.at.Format("2006-01-02T15:04:05.999999999-07:00")
			}
			for _, key := range []string{"time", "created_at", "updated_at", "closed_at", "last_read_at"} {
				if _, ok := offset[key]; ok {
					offset[key] = stamp
				}
			}
			if comments, ok := offset["comments"].([]any); ok {
				for _, c := range comments {
					c.(map[string]any)["created_at"] = stamp
				}
			}
			add(tc.name, offset)
		}
		if name == "Task" || name == "Memory" {
			for name, value := range map[string]any{"metadata_null": nil, "metadata_empty": map[string]any{}, "metadata_false": false} {
				metadata := object(record)
				metadata["metadata"] = value
				add(name, metadata)
			}
		}
		if name == "Task" {
			widePriority := object(record)
			widePriority["priority"] = int64(9007199254740993)
			add("wide_priority", widePriority)
		}
		fixtures[name] = cases
	}
	b, err := json.MarshalIndent(fixtures, "", "  ")
	must(err)
	must(os.WriteFile(*out, append(b, '\n'), 0600))
}
