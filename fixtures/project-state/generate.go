// SPDX-License-Identifier: GPL-3.0-only
// Run from repos/sdk: GOROOT=/usr/local/go GOTOOLCHAIN=local go run ../../fixtures/project-state/generate.go -out ../../fixtures/project-state
package main

import (
	"bufio"
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	mem "github.com/gratefulagents/sdk/pkg/agentsdk/memory"
	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
	_ "modernc.org/sqlite"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func decode(b []byte, v any) {
	d := json.NewDecoder(bytes.NewReader(b))
	d.UseNumber()
	must(d.Decode(v))
}
func value(v any) any { b, e := json.Marshal(v); must(e); var out any; decode(b, &out); return out }
func write(path string, v any) {
	b, e := json.MarshalIndent(v, "", "  ")
	must(e)
	must(os.WriteFile(path, append(b, '\n'), 0600))
}
func ptr[T any](v T) *T { return &v }

type normalizer struct {
	ids    map[string]string
	times  map[string]string
	counts map[string]int
	temp   string
}

func (n *normalizer) walk(v any) any {
	switch x := v.(type) {
	case map[string]any:
		keys := make([]string, 0, len(x))
		for k := range x {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		for _, k := range keys {
			v := n.walk(x[k])
			key := n.walk(k).(string)
			if key != k {
				delete(x, k)
			}
			x[key] = v
		}
		return x
	case []any:
		for i := range x {
			x[i] = n.walk(x[i])
		}
		return x
	case string:
		if strings.HasPrefix(x, n.temp) {
			return strings.Replace(x, n.temp, "/fixture", 1)
		}
		if t, e := time.Parse(time.RFC3339Nano, x); e == nil && !t.IsZero() {
			if out, ok := n.times[x]; ok {
				return out
			}
			out := time.Date(2025, 1, 1, 0, 0, len(n.times), 123456789, time.UTC).Format(time.RFC3339Nano)
			n.times[x] = out
			return out
		}
		for _, prefix := range []string{"task_", "mem_", "session_", "comment_", "evt_"} {
			if strings.HasPrefix(x, prefix) {
				if out, ok := n.ids[x]; ok {
					return out
				}
				n.counts[prefix]++
				out := fmt.Sprintf("%s%012d", prefix, n.counts[prefix])
				n.ids[x] = out
				return out
			}
		}
		return x
	default:
		return x
	}
}
func main() {
	out := flag.String("out", ".", "output directory")
	flag.Parse()
	must(os.MkdirAll(*out, 0700))
	tmp, e := os.MkdirTemp("", "project-state-go-")
	must(e)
	defer os.RemoveAll(tmp)
	ctx := context.Background()
	stateDir := filepath.Join(tmp, "state")
	s, e := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: stateDir, ProjectID: "Fixture Project", WorkDir: filepath.Join(tmp, "work"), Actor: "go-agent", RunID: "fixture-run", Embedder: fixtureEmbedder{}})
	must(e)
	a, e := s.CreateTask(ctx, ps.CreateTaskInput{Title: " Foundation ", Type: "feat", Priority: 9, Labels: []string{"Core", "Core", ""}, Metadata: json.RawMessage(`{"large":9007199254740993,"nested":{"ok":true}}`)})
	must(e)
	b, e := s.CreateTask(ctx, ps.CreateTaskInput{Title: "Dependent", Priority: 1, Description: "Depends on foundation", DependsOn: []string{a.ID}})
	must(e)
	_, e = s.AddComment(ctx, a.ID, "reviewer", " comment ")
	must(e)
	_, e = s.UpdateTask(ctx, a.ID, ps.TaskPatch{Priority: ptr(2), Labels: []string{"Core", "Ready"}, ReplaceLabels: true})
	must(e)
	must(s.RemoveDependency(ctx, b.ID, a.ID))
	must(s.AddDependency(ctx, b.ID, a.ID))
	_, e = s.ClaimTask(ctx, a.ID, "")
	must(e)
	_, e = s.CloseTask(ctx, a.ID, "implemented")
	must(e)
	pinned, e := s.UpsertMemory(ctx, ps.UpsertMemoryInput{Kind: "pinned", Content: "Rust uses ownership", Tags: []string{"rust", "design"}, TaskIDs: []string{a.ID}, FilePaths: []string{"src/lib.rs"}, Metadata: json.RawMessage(`{"public":true}`)})
	must(e)
	_, e = s.UpsertMemory(ctx, ps.UpsertMemoryInput{ID: pinned.ID, Kind: "pinned", Content: "Rust uses ownership and borrowing", Tags: []string{"rust", "design"}, TaskIDs: []string{a.ID}, FilePaths: []string{"src/lib.rs"}})
	must(e)
	_, e = s.UpsertMemory(ctx, ps.UpsertMemoryInput{Kind: "procedural", Scope: "file", Content: "Run cargo test before shipping", Tags: []string{"rust", "tests"}})
	must(e)
	deleted, e := s.UpsertMemory(ctx, ps.UpsertMemoryInput{Content: "Delete this historical value"})
	must(e)
	must(s.DeleteMemory(ctx, deleted.ID))
	_, e = s.SaveSessionSummary(ctx, ps.SessionSummary{Summary: "Foundation delivered", TaskIDs: []string{a.ID, b.ID}})
	must(e)
	tasks, e := s.ListTasks(ctx)
	must(e)
	memories, e := s.ListMemories(ctx, ps.MemoryFilter{})
	must(e)
	sessions, e := s.ListSessionSummaries(ctx, 0)
	must(e)
	ready, e := s.ReadyTasks(ctx, ps.TaskFilter{})
	must(e)
	recall, e := s.SearchMemories(ctx, ps.MemoryFilter{Query: "ownership missing"})
	must(e)
	expected := value(map[string]any{"tasks": tasks, "memories": memories, "sessions": sessions, "ready": ready, "recall": recall})
	n := normalizer{ids: map[string]string{}, times: map[string]string{}, counts: map[string]int{}, temp: tmp}
	baseline := filepath.Join(*out, "baseline")
	must(os.MkdirAll(filepath.Join(baseline, "indexes"), 0700))
	file, e := os.Open(filepath.Join(stateDir, "events.jsonl"))
	must(e)
	scanner := bufio.NewScanner(file)
	var events []ps.Event
	dest, e := os.OpenFile(filepath.Join(baseline, "events.jsonl"), os.O_CREATE|os.O_TRUNC|os.O_WRONLY, 0600)
	must(e)
	for scanner.Scan() {
		var v any
		decode(scanner.Bytes(), &v)
		raw, e := json.Marshal(n.walk(v))
		must(e)
		_, e = dest.Write(append(raw, '\n'))
		must(e)
		var ev ps.Event
		must(json.Unmarshal(raw, &ev))
		events = append(events, ev)
	}
	must(scanner.Err())
	must(file.Close())
	must(dest.Close())
	// Re-open through Go after normalizing; outputs are computed from the actual replay contract.
	normalized, e := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: baseline, ProjectID: "fixture-project"})
	must(e)
	tasks, e = normalized.ListTasks(ctx)
	must(e)
	memories, e = normalized.ListMemories(ctx, ps.MemoryFilter{})
	must(e)
	sessions, e = normalized.ListSessionSummaries(ctx, 0)
	must(e)
	ready, e = normalized.ReadyTasks(ctx, ps.TaskFilter{})
	must(e)
	recall, e = normalized.SearchMemories(ctx, ps.MemoryFilter{Query: "ownership missing"})
	must(e)
	expected = value(map[string]any{"tasks": tasks, "memories": memories, "sessions": sessions, "ready": ready, "recall": recall})
	write(filepath.Join(*out, "expected.json"), expected)
	prime, e := normalized.PrimeContext(ctx, ps.PrimeOptions{})
	must(e)
	must(os.WriteFile(filepath.Join(*out, "prime.txt"), []byte(prime+"\n"), 0600))
	must(normalized.Close())
	must(s.Close())
	for _, name := range []string{"project", "tasks", "memories", "sessions", "embeddings"} {
		raw, e := os.ReadFile(filepath.Join(stateDir, "indexes", name+".json"))
		must(e)
		var v any
		decode(raw, &v)
		write(filepath.Join(baseline, "indexes", name+".json"), n.walk(v))
	}
	dbPath := filepath.Join(*out, "baseline.sqlite")
	must(os.RemoveAll(dbPath))
	sq, e := ps.NewSQLiteStore(ps.SQLiteOptions{Path: dbPath, ProjectID: "fixture-project"})
	must(e)
	must(sq.Close())
	db, e := sql.Open("sqlite", dbPath)
	must(e)
	_, e = db.Exec("DELETE FROM projectstate_events")
	must(e)
	for _, ev := range events {
		_, e = db.Exec("INSERT INTO projectstate_events(project_id,seq,event_id,run_id,actor,ts,type,payload) VALUES(?,?,?,?,?,?,?,?)", ev.ProjectID, ev.Seq, ev.EventID, ev.RunID, ev.Actor, ev.Time.UnixNano(), ev.Type, []byte(ev.Payload))
		must(e)
	}
	must(db.Close())
	sq, e = ps.NewSQLiteStore(ps.SQLiteOptions{Path: dbPath, ProjectID: "fixture-project", Embedder: fixtureEmbedder{}})
	must(e)
	sqlTasks, e := sq.ListTasks(ctx)
	must(e)
	if stringJSON(tasks) != stringJSON(sqlTasks) {
		panic("SQLite replay mismatch")
	}
	_, e = sq.SearchMemories(ctx, ps.MemoryFilter{Query: "ownership"})
	must(e)
	must(sq.Close())
	local := mem.NewInMemoryStore()
	one, e := local.Store(ctx, "project-a", "alpha beta", []string{"one"}, "fixture-run", nil)
	must(e)
	_, e = local.Store(ctx, "project-a", "beta and alpha", []string{"two"}, "", nil)
	must(e)
	_, e = local.Store(ctx, "project-b", "alpha beta", nil, "", nil)
	must(e)
	hits, e := local.Search(ctx, "project-a", "alpha beta", []string{"one", "two"}, 10)
	must(e)
	for i := range hits {
		hits[i].ID[15] = byte(i + 1)
		for j := 0; j < 15; j++ {
			hits[i].ID[j] = 0
		}
		hits[i].CreatedAt = time.Date(2025, 1, 1, 0, i, 0, 0, time.UTC)
	}
	_ = one
	write(filepath.Join(*out, "namespace-memory.json"), hits)
	write(filepath.Join(*out, "manifest.json"), map[string]any{"source": "github.com/gratefulagents/sdk", "revision": "1dc92b73900fac74dc357a938e4b5eee6392b418", "packages": []string{"pkg/agentsdk/projectstate", "pkg/agentsdk/memory"}, "schema_version": ps.SchemaVersion, "license": "GPL-3.0-only", "normalization": "IDs, timestamps, and temporary directory paths only; Go API output and replay determine expected behavior"})
}
func stringJSON(v any) string { b, e := json.Marshal(v); must(e); return string(b) }

type fixtureEmbedder struct{}

func (fixtureEmbedder) Model() string { return "fixture-model" }
func (fixtureEmbedder) Embed(_ context.Context, texts []string) ([][]float32, error) {
	out := make([][]float32, len(texts))
	for i, text := range texts {
		if strings.Contains(text, "ownership") {
			out[i] = []float32{1, 0}
		} else {
			out[i] = []float32{0, 1}
		}
	}
	return out, nil
}
