// SPDX-License-Identifier: GPL-3.0-only
package agentsdk

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"math"
	"os"
	"strings"
	"testing"
	"time"
)

type nativeFailedContentWriter struct{ attempts int }

func (w *nativeFailedContentWriter) Write(p []byte) (int, error) {
	w.attempts++
	return 0, io.ErrClosedPipe
}

func TestNativeContentStreamReference(t *testing.T) {
	start := time.Now()
	record := func(event ContentEvent) map[string]any {
		automatic := !event.Timestamp.Before(start)
		if automatic {
			_, offset := event.Timestamp.Zone()
			if offset != 0 {
				t.Fatal("expected UTC reference profile")
			}
			if event.Timestamp.After(time.Now()) {
				t.Fatal("future automatic timestamp")
			}
			event.Timestamp = time.Time{}
		}
		nonfinite := math.IsNaN(event.CostUsd)
		if nonfinite {
			event.CostUsd = 0
		}
		encoded, err := json.Marshal(event)
		if err != nil {
			t.Fatal(err)
		}
		return map[string]any{"automatic": automatic, "nonfinite": nonfinite, "json": string(encoded)}
	}
	drain := func(ch <-chan ContentEvent) map[string]any {
		events := []map[string]any{}
		closed := false
	loop:
		for {
			select {
			case event, ok := <-ch:
				if !ok {
					closed = true
					break loop
				}
				events = append(events, record(event))
			default:
				break loop
			}
		}
		return map[string]any{"events": events, "closed": closed}
	}
	wire := func(buf *bytes.Buffer) []map[string]any {
		events := []map[string]any{}
		for _, line := range strings.Split(strings.TrimSpace(buf.String()), "\n") {
			if line == "" {
				continue
			}
			var event ContentEvent
			if err := json.Unmarshal([]byte(line), &event); err != nil {
				t.Fatal(err)
			}
			events = append(events, record(event))
		}
		return events
	}
	var output bytes.Buffer
	root := NewEventStream(&output)
	root.SetSession(7)
	root.SetPhase("planning")
	root.SetStep("exploring")
	child := NewChildEventStream(root, "task")
	root.SetSession(8)
	root.SetPhase("implementation")
	root.SetStep("editing")
	sub, unsubscribe := child.Subscribe(16)
	root.Emit(ContentEvent{Type: "root", Message: "message"})
	child.Emit(ContentEvent{Type: "child"})
	fixed := time.Date(2020, 2, 3, 4, 5, 6, 123400000, time.FixedZone("east", 19800))
	child.Emit(ContentEvent{Type: "explicit", Session: 9, Phase: "phase", Step: "step", TaskID: "override", Timestamp: fixed})
	child.Emit(ContentEvent{Type: "zero_offset", Timestamp: time.Date(1, 1, 1, 1, 0, 0, 0, time.FixedZone("east", 3600))})
	child.Emit(ContentEvent{Type: "zero_large_offset", Timestamp: time.Date(1, 1, 2, 1, 0, 0, 0, time.FixedZone("east", 90000))})
	root.SetSession(0)
	root.SetPhase("")
	root.SetStep("")
	root.Emit(ContentEvent{Type: "cleared"})
	root.SetSession(-2147483648)
	root.SetPhase("界\nphase")
	root.SetStep(" \tstep")
	root.Emit(ContentEvent{Type: "minimum"})
	root.SetSession(2147483647)
	root.Emit(ContentEvent{Type: "maximum"})
	metadata := map[string]any{"subscriber": drain(sub), "wire": wire(&output)}
	unsubscribe()
	unsubscribe()
	root.Emit(ContentEvent{Type: "after_close"})
	metadata["closed"] = drain(sub)

	output.Reset()
	root = NewEventStream(&output)
	child = NewChildEventStream(root, "task")
	for _, event := range []ContentEvent{
		{Type: "subagent_status", TaskID: "task", SubagentType: "worker", AgentName: "explicit"},
		{Type: "subagent_status"},
		{Type: "subagent_status", AgentName: "renamed"},
		{Type: "subagent_status"},
		{Type: "other"},
		{Type: "subagent_status", TaskID: "unknown"},
	} {
		child.Emit(event)
	}
	annotation := wire(&output)

	drops := []map[string]any{}
	for _, capacity := range []int{1, 2, 0, -1} {
		output.Reset()
		root = NewEventStream(&output)
		sub, unsubscribe = root.Subscribe(capacity)
		count := 4
		if capacity <= 0 {
			count = 66
		}
		for i := 0; i < count; i++ {
			root.Emit(ContentEvent{Type: "tick", Message: fmt.Sprint(i)})
		}
		first := drain(sub)
		root.Emit(ContentEvent{Type: "later"})
		second := drain(sub)
		root.Emit(ContentEvent{Type: "last"})
		third := drain(sub)
		unsubscribe()
		drops = append(drops, map[string]any{"capacity": capacity, "first": first, "second": second, "third": third, "closed": drain(sub)})
	}

	failed := &nativeFailedContentWriter{}
	root = NewEventStream(failed)
	sub, unsubscribe = root.Subscribe(4)
	root.Emit(ContentEvent{Type: "writer_failure"})
	root.Emit(ContentEvent{Type: "marshal_failure", CostUsd: math.NaN()})
	failure := map[string]any{"attempts": failed.attempts, "subscriber": drain(sub)}
	unsubscribe()
	result := map[string]any{"metadata": metadata, "annotation": annotation, "drops": drops, "failure": failure}
	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("METADATA_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
