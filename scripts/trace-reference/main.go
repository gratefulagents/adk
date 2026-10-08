package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/gratefulagents/sdk/pkg/agentsdk/tracestore"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func main() {
	if len(os.Args) > 1 && os.Args[1] == "stdout" {
		stdoutFixture()
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "otel" {
		otelFixture()
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "writer" {
		writerFixture()
		return
	}
	root, err := os.MkdirTemp("", "adk-trace-reference-")
	must(err)
	defer os.RemoveAll(root)
	store, err := tracestore.NewFilesystemTraceStore(root)
	must(err)
	defer store.Close()
	start := time.Date(2025, 1, 2, 3, 4, 5, 123000000, time.UTC)
	path, err := store.CreateRunDir("run", tracestore.RunMetadata{RunID: "run", CandidateID: "candidate", StartedAt: start})
	must(err)
	must(store.AppendTrace("run", "tool_calls", []byte(`{"schema_version":2,"run_id":"run","type":"tool_start"}`)))
	must(store.WriteFile("run", "artifacts/result.txt", []byte("result\n")))
	must(store.WriteScore("run", tracestore.Score{TaskID: "task", CandidateID: "candidate", Success: true, Metrics: tracestore.ScoreMetrics{Accuracy: 1, TokensUsed: 42}}))
	must(store.UpdateMetadataMode("run", "plan"))
	must(store.UpdateMetadataFinishedAt("run", start.Add(time.Second)))
	files := map[string]any{}
	for _, name := range []string{"metadata.json", "score.json"} {
		data, err := os.ReadFile(filepath.Join(path, name))
		must(err)
		var document any
		must(json.Unmarshal(data, &document))
		files[name] = document
	}
	for _, name := range []string{"tool_calls.jsonl", "artifacts/result.txt"} {
		data, err := os.ReadFile(filepath.Join(path, name))
		must(err)
		files[name] = string(data)
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(files))
}
