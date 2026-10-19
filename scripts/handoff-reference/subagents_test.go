package main

import (
	"bytes"
	"encoding/json"
	"testing"
)

func TestPublicBuilderSubagentSelections(t *testing.T) {
	first, err := generateSubagents()
	if err != nil {
		t.Fatal(err)
	}
	second, err := generateSubagents()
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first, second) {
		t.Fatal("public builder projection is not deterministic")
	}
	var result struct {
		Cases []subagentSelectionCase `json:"cases"`
	}
	if err := json.Unmarshal(first, &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Cases) != 8 {
		t.Fatalf("got %d cases", len(result.Cases))
	}
	seen := map[subagentSelection]bool{}
	for _, c := range result.Cases {
		if seen[c.Selection] {
			t.Fatal("duplicate selection")
		}
		seen[c.Selection] = true
		if c.Scheduler != (len(c.Tools) > 0) {
			t.Fatal("scheduler/tool selection mismatch")
		}
	}
	if len(result.Cases[0].Tools) != 0 || len(result.Cases[7].Tools) != 4 {
		t.Fatal("wrong all-off/all-on surfaces")
	}
}
