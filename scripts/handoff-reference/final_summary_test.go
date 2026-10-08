package main

import (
	"bytes"
	"encoding/json"
	"testing"
)

func TestFinalSummaryRunnerProjection(t *testing.T) {
	first, err := generateFinalSummary()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generateFinalSummary()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatal("runner projection is not deterministic")
	}
	var result struct {
		Cases []summaryCase `json:"cases"`
	}
	if err := json.Unmarshal(first, &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Cases) != 8 {
		t.Fatalf("got %d cases", len(result.Cases))
	}
	for _, c := range result.Cases {
		if len(c.Requests) != c.Turns || c.ToolCalls != c.Turns-1 || c.Output != "summary" {
			t.Fatalf("unexpected runner result: %+v", c)
		}
		if (len(c.Requests[len(c.Requests)-1].Tools) == 0) != c.Enabled {
			t.Fatal("unexpected final tool surface")
		}
	}
}
