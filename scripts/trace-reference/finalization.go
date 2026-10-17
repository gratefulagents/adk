package main

import (
	"encoding/json"
	"errors"
	"os"
	"time"

	"github.com/gratefulagents/sdk/pkg/agentsdk/tracestore"
)

// Embedding leaves unrelated methods unavailable: this oracle only exercises
// the three independent FinalizeRun persistence operations.
type finalizationStore struct {
	tracestore.TraceStore
	mask     int
	calls    []string
	ended    bool
	terminal map[string]any
	started  time.Time
}

func (s *finalizationStore) AppendTrace(runID, category string, data []byte) error {
	s.calls = append(s.calls, "append:"+category)
	must(json.Unmarshal(data, &s.terminal))
	delete(s.terminal, "timestamp")
	if s.mask&1 != 0 {
		return errors.New("append failure")
	}
	return nil
}
func (s *finalizationStore) WriteFile(runID, path string, data []byte) error {
	s.calls = append(s.calls, "write:"+path)
	if s.mask&2 != 0 {
		return errors.New("health failure")
	}
	return nil
}
func (s *finalizationStore) UpdateMetadataFinishedAt(runID string, finished time.Time) error {
	s.calls = append(s.calls, "finish")
	if finished.Before(s.started) || finished.After(time.Now()) {
		panic("invalid finalization time")
	}
	if s.mask&4 != 0 {
		return errors.New("metadata failure")
	}
	s.ended = true
	return nil
}
func finalizationFixture() {
	cases := []map[string]any{}
	for _, runID := range []string{"run", ""} {
		for mask := 0; mask < 8; mask++ {
			store := &finalizationStore{mask: mask, calls: []string{}, started: time.Now()}
			writer := tracestore.NewTraceWriter(store)
			writer.SetRunID(runID)
			writer.FinalizeRun("finished")
			cases = append(cases, map[string]any{"mask": mask, "runID": runID, "calls": store.calls, "health": writer.Health(), "ended": store.ended, "terminal": store.terminal})
		}
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	must(encoder.Encode(cases))
}
