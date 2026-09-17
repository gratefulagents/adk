// SPDX-License-Identifier: GPL-3.0-only
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"

	ps "github.com/gratefulagents/sdk/pkg/agentsdk/projectstate"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func decode(data []byte) any {
	var value any
	d := json.NewDecoder(bytes.NewReader(data))
	d.UseNumber()
	must(d.Decode(&value))
	return value
}
func main() {
	if len(os.Args) != 2 {
		panic("usage: go run verify/main.go RUST_EXPORT_DIR")
	}
	ctx := context.Background()
	for _, backend := range []string{"filesystem", "sqlite"} {
		dir := filepath.Join(os.Args[1], backend)
		var store ps.Store
		if backend == "filesystem" {
			s, e := ps.NewFilesystemStore(ps.FilesystemOptions{StateDir: filepath.Join(dir, "state"), ProjectID: "rust-roundtrip"})
			must(e)
			store = s
		} else {
			s, e := ps.NewSQLiteStore(ps.SQLiteOptions{Path: filepath.Join(dir, "state.db"), ProjectID: "rust-roundtrip"})
			must(e)
			store = s
		}
		tasks, e := store.ListTasks(ctx)
		must(e)
		memories, e := store.ListMemories(ctx, ps.MemoryFilter{})
		must(e)
		sessions, e := store.ListSessionSummaries(ctx, 0)
		must(e)
		ready, e := store.ReadyTasks(ctx, ps.TaskFilter{})
		must(e)
		prime, e := store.PrimeContext(ctx, ps.PrimeOptions{})
		must(e)
		actual, e := json.Marshal(map[string]any{"tasks": tasks, "memories": memories, "sessions": sessions, "ready": ready, "prime": prime})
		must(e)
		expected, e := os.ReadFile(filepath.Join(dir, "expected.json"))
		must(e)
		if !reflect.DeepEqual(decode(actual), decode(expected)) {
			panic(fmt.Sprintf("%s Rust -> Go replay mismatch\nactual=%s\nexpected=%s", backend, actual, expected))
		}
		_, e = store.CreateTask(ctx, ps.CreateTaskInput{Title: "Go appended after Rust"})
		must(e)
		must(store.Close())
		fmt.Printf("PASS Rust -> Go %s replay, schemas, priming, and subsequent Go write\n", backend)
	}
}
